use std::sync::{Arc, RwLock};

use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::{AngularJsAnalyzer, HtmlParser, JsParser};
use crate::config::InterpolateConfig;
use crate::index::{BindingSource, HtmlControllerScope, HtmlScopeReference, NgIncludeBinding, SymbolIndex, SymbolReference, TemplateBinding};

/// HTML内のAngularJSディレクティブを解析するアナライザー
pub struct HtmlAngularJsAnalyzer {
    index: Arc<SymbolIndex>,
    js_analyzer: Arc<AngularJsAnalyzer>,
    interpolate: RwLock<InterpolateConfig>,
}

impl HtmlAngularJsAnalyzer {
    pub fn new(index: Arc<SymbolIndex>, js_analyzer: Arc<AngularJsAnalyzer>) -> Self {
        Self {
            index,
            js_analyzer,
            interpolate: RwLock::new(InterpolateConfig::default()),
        }
    }

    /// interpolate設定を更新
    pub fn set_interpolate_config(&self, config: InterpolateConfig) {
        if let Ok(mut interpolate) = self.interpolate.write() {
            *interpolate = config;
        }
    }

    /// 現在のinterpolate設定を取得
    fn get_interpolate_symbols(&self) -> (String, String) {
        if let Ok(config) = self.interpolate.read() {
            (config.start_symbol.clone(), config.end_symbol.clone())
        } else {
            ("{{".to_string(), "}}".to_string())
        }
    }

    /// HTMLドキュメントを解析
    pub fn analyze_document(&self, uri: &Url, source: &str) {
        let mut parser = HtmlParser::new();

        if let Some(tree) = parser.parse(source) {
            // 既存のHTML情報をクリア
            self.index.clear_document(uri);

            // Pass 1: ng-controllerスコープとng-includeを収集
            let mut controller_stack: Vec<String> = Vec::new();
            self.collect_controller_scopes_and_includes(tree.root_node(), source, uri, &mut controller_stack);

            // Pass 2: <script>タグ内のJSを解析してバインディングを抽出
            self.analyze_script_tags(tree.root_node(), source, uri);

            // Pass 3: $scope参照を収集
            self.collect_scope_references(tree.root_node(), source, uri);
        }
    }

    /// ng-controllerスコープとng-includeを収集
    fn collect_controller_scopes_and_includes(
        &self,
        node: Node,
        source: &str,
        uri: &Url,
        controller_stack: &mut Vec<String>,
    ) {
        // 要素ノードの場合、ng-controller属性をチェック
        if node.kind() == "element" {
            let mut added_controller = false;

            // 開始タグから属性を取得
            if let Some(start_tag) = self.find_child_by_kind(node, "start_tag") {
                // ng-controllerをチェック
                if let Some(controller_name) = self.get_ng_controller_attribute(start_tag, source) {
                    // ng-controllerスコープを登録
                    let scope = HtmlControllerScope {
                        controller_name: controller_name.clone(),
                        uri: uri.clone(),
                        start_line: node.start_position().row as u32,
                        end_line: node.end_position().row as u32,
                    };
                    self.index.add_html_controller_scope(scope);
                    controller_stack.push(controller_name);
                    added_controller = true;
                }

                // ng-includeをチェック
                if let Some(template_path) = self.get_ng_include_attribute(start_tag, source) {
                    // 親ファイルを起点として相対パスを解決
                    let resolved_filename = crate::index::SymbolIndex::resolve_relative_path(uri, &template_path);
                    // 現在のコントローラースタックをコピーして継承情報として登録
                    let binding = NgIncludeBinding {
                        parent_uri: uri.clone(),
                        template_path,
                        resolved_filename,
                        line: node.start_position().row as u32,
                        inherited_controllers: controller_stack.clone(),
                    };
                    self.index.add_ng_include_binding(binding);
                }
            }

            // 子要素を再帰的に処理
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.collect_controller_scopes_and_includes(child, source, uri, controller_stack);
            }

            // このノードで追加したコントローラーをスタックから削除
            if added_controller {
                controller_stack.pop();
            }
        } else {
            // 子ノードを再帰的に処理
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.collect_controller_scopes_and_includes(child, source, uri, controller_stack);
            }
        }
    }

    /// ng-controller属性の値を取得
    fn get_ng_controller_attribute(&self, start_tag: Node, source: &str) -> Option<String> {
        let mut cursor = start_tag.walk();
        for child in start_tag.children(&mut cursor) {
            if child.kind() == "attribute" {
                if let Some(name) = self.find_child_by_kind(child, "attribute_name") {
                    let attr_name = self.node_text(name, source);
                    if attr_name == "ng-controller" || attr_name == "data-ng-controller" {
                        if let Some(value) = self.find_child_by_kind(child, "quoted_attribute_value") {
                            let raw_value = self.node_text(value, source);
                            // クォートを除去し、"Controller as alias"の場合はControllerだけ取得
                            let value = raw_value.trim_matches(|c| c == '"' || c == '\'');
                            let controller_name = value.split_whitespace().next().unwrap_or(value);
                            return Some(controller_name.to_string());
                        }
                    }
                }
            }
        }
        None
    }

    /// ng-include属性またはsrc属性（<ng-include>要素用）の値を取得
    fn get_ng_include_attribute(&self, start_tag: Node, source: &str) -> Option<String> {
        // タグ名を取得
        let tag_name_node = self.find_child_by_kind(start_tag, "tag_name");
        let tag_name = tag_name_node.map(|n| self.node_text(n, source));

        let mut cursor = start_tag.walk();
        for child in start_tag.children(&mut cursor) {
            if child.kind() == "attribute" {
                if let Some(name) = self.find_child_by_kind(child, "attribute_name") {
                    let attr_name = self.node_text(name, source);

                    // ng-include属性または<ng-include>要素のsrc属性をチェック
                    let is_ng_include = attr_name == "ng-include" || attr_name == "data-ng-include";
                    let is_ng_include_src = (tag_name.as_deref() == Some("ng-include") || tag_name.as_deref() == Some("data-ng-include"))
                        && attr_name == "src";

                    if is_ng_include || is_ng_include_src {
                        if let Some(value) = self.find_child_by_kind(child, "quoted_attribute_value") {
                            let raw_value = self.node_text(value, source);
                            // 外側のクォート（最初と最後の1文字）を除去
                            // 例: "'child.html'" -> "'child.html'"
                            // 例: "\"'child.html'\"" -> "'child.html'"
                            let value = if raw_value.len() >= 2 {
                                &raw_value[1..raw_value.len() - 1]
                            } else {
                                raw_value.as_str()
                            };
                            // 文字列リテラル部分を抽出
                            // 例: "'template.html'" -> "template.html"
                            // 例: "'path/to/file.html?' + version" -> "path/to/file.html?"
                            if let Some(template_path) = self.extract_string_literal(value) {
                                return Some(template_path);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// 式から最初の文字列リテラルを抽出
    /// 例: "'template.html'" -> "template.html"
    /// 例: "'path/to/file.html?' + version" -> "path/to/file.html?"
    fn extract_string_literal(&self, expr: &str) -> Option<String> {
        let expr = expr.trim();

        // シングルクォートで始まる場合
        if expr.starts_with('\'') {
            if let Some(end_idx) = expr[1..].find('\'') {
                return Some(expr[1..end_idx + 1].to_string());
            }
        }

        // ダブルクォートで始まる場合
        if expr.starts_with('"') {
            if let Some(end_idx) = expr[1..].find('"') {
                return Some(expr[1..end_idx + 1].to_string());
            }
        }

        None
    }

    /// <script>タグ内のJavaScriptを解析
    fn analyze_script_tags(&self, node: Node, source: &str, uri: &Url) {
        if node.kind() == "script_element" {
            // <script>タグの内容を取得
            if let Some(raw_text) = self.find_child_by_kind(node, "raw_text") {
                let js_source = self.node_text(raw_text, source);
                // scriptタグの開始行をオフセットとして使用
                let line_offset = raw_text.start_position().row as u32;

                // AngularJsAnalyzerで完全な解析を実行
                self.js_analyzer.analyze_embedded_script(uri, &js_source, line_offset);

                // テンプレートバインディングも抽出
                self.analyze_embedded_js(&js_source);
            }
        }

        // 子ノードを再帰的に処理
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.analyze_script_tags(child, source, uri);
        }
    }

    /// 埋め込みJavaScriptからテンプレートバインディングを抽出
    fn analyze_embedded_js(&self, js_source: &str) {
        let mut parser = JsParser::new();
        if let Some(tree) = parser.parse(js_source) {
            self.extract_bindings_from_js(tree.root_node(), js_source);
        }
    }

    /// JSのASTからテンプレートバインディングを抽出
    fn extract_bindings_from_js(&self, node: Node, source: &str) {
        if node.kind() == "call_expression" {
            if let Some(callee) = node.child_by_field_name("function") {
                if callee.kind() == "member_expression" {
                    if let Some(property) = callee.child_by_field_name("property") {
                        let method_name = self.node_text(property, source);
                        match method_name.as_str() {
                            "when" => self.extract_route_binding_js(node, callee, source),
                            "open" => self.extract_modal_binding_js(node, callee, source),
                            _ => {}
                        }
                    }
                }
            }
        }

        // 子ノードを再帰的に処理
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.extract_bindings_from_js(child, source);
        }
    }

    /// $routeProvider.when()からバインディングを抽出
    fn extract_route_binding_js(&self, node: Node, callee: Node, source: &str) {
        if let Some(object) = callee.child_by_field_name("object") {
            let obj_text = self.node_text(object, source);
            if !obj_text.ends_with("routeProvider") && !obj_text.ends_with("$routeProvider") {
                if object.kind() != "call_expression" {
                    return;
                }
            }
        }

        if let Some(args) = node.child_by_field_name("arguments") {
            if let Some(config_obj) = args.named_child(1) {
                if config_obj.kind() == "object" {
                    self.extract_template_binding_from_js_object(config_obj, source, BindingSource::RouteProvider);
                }
            }
        }
    }

    /// $uibModal.open()からバインディングを抽出
    fn extract_modal_binding_js(&self, node: Node, callee: Node, source: &str) {
        if let Some(object) = callee.child_by_field_name("object") {
            let obj_text = self.node_text(object, source);
            if !obj_text.ends_with("Modal") && !obj_text.ends_with("$uibModal") && !obj_text.ends_with("$modal") {
                return;
            }
        }

        if let Some(args) = node.child_by_field_name("arguments") {
            if let Some(config_obj) = args.named_child(0) {
                if config_obj.kind() == "object" {
                    self.extract_template_binding_from_js_object(config_obj, source, BindingSource::UibModal);
                }
            }
        }
    }

    /// JSオブジェクトからcontrollerとtemplateUrlを抽出
    fn extract_template_binding_from_js_object(&self, obj_node: Node, source: &str, binding_source: BindingSource) {
        let mut controller_name: Option<String> = None;
        let mut template_url: Option<String> = None;

        let mut cursor = obj_node.walk();
        for child in obj_node.children(&mut cursor) {
            if child.kind() == "pair" {
                if let Some(key) = child.child_by_field_name("key") {
                    let key_name = self.node_text(key, source);
                    let key_name = key_name.trim_matches(|c| c == '"' || c == '\'');

                    if let Some(value) = child.child_by_field_name("value") {
                        match key_name {
                            "controller" => {
                                if value.kind() == "string" {
                                    controller_name = Some(self.extract_string_value(value, source));
                                }
                            }
                            "templateUrl" => {
                                if value.kind() == "string" {
                                    template_url = Some(self.extract_string_value(value, source));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        if let (Some(controller), Some(template)) = (controller_name, template_url) {
            let binding = TemplateBinding {
                template_path: template,
                controller_name: controller,
                source: binding_source,
            };
            self.index.add_template_binding(binding);
        }
    }

    /// $scope参照を収集
    fn collect_scope_references(&self, node: Node, source: &str, uri: &Url) {
        // 要素ノードの場合、AngularJSディレクティブをチェック
        if node.kind() == "element" {
            if let Some(start_tag) = self.find_child_by_kind(node, "start_tag") {
                self.extract_scope_references_from_tag(start_tag, source, uri);
            }
        }

        // text内の{{interpolation}}をチェック
        if node.kind() == "text" {
            let text = self.node_text(node, source);
            self.extract_interpolation_references(&text, node, source, uri);
        }

        // 子ノードを再帰的に処理
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.collect_scope_references(child, source, uri);
        }
    }

    /// タグの属性からスコープ参照を抽出
    fn extract_scope_references_from_tag(&self, start_tag: Node, source: &str, uri: &Url) {
        let mut cursor = start_tag.walk();
        for child in start_tag.children(&mut cursor) {
            if child.kind() == "attribute" {
                if let Some(name_node) = self.find_child_by_kind(child, "attribute_name") {
                    let attr_name = self.node_text(name_node, source);

                    // サポートするAngularJSディレクティブ
                    let is_ng_directive = matches!(
                        attr_name.as_str(),
                        // データバインディング
                        "ng-model" | "data-ng-model" |
                        "ng-bind" | "data-ng-bind" |
                        "ng-bind-html" | "data-ng-bind-html" |
                        "ng-value" | "data-ng-value" |
                        "ng-init" | "data-ng-init" |
                        // 条件・繰り返し
                        "ng-if" | "data-ng-if" |
                        "ng-show" | "data-ng-show" |
                        "ng-hide" | "data-ng-hide" |
                        "ng-repeat" | "data-ng-repeat" |
                        "ng-switch" | "data-ng-switch" |
                        "ng-switch-when" | "data-ng-switch-when" |
                        // スタイル・クラス
                        "ng-class" | "data-ng-class" |
                        "ng-style" | "data-ng-style" |
                        // フォームバリデーション
                        "ng-disabled" | "data-ng-disabled" |
                        "ng-checked" | "data-ng-checked" |
                        "ng-selected" | "data-ng-selected" |
                        "ng-readonly" | "data-ng-readonly" |
                        "ng-required" | "data-ng-required" |
                        "ng-pattern" | "data-ng-pattern" |
                        "ng-minlength" | "data-ng-minlength" |
                        "ng-maxlength" | "data-ng-maxlength" |
                        // イベントハンドラ
                        "ng-click" | "data-ng-click" |
                        "ng-dblclick" | "data-ng-dblclick" |
                        "ng-change" | "data-ng-change" |
                        "ng-submit" | "data-ng-submit" |
                        "ng-blur" | "data-ng-blur" |
                        "ng-focus" | "data-ng-focus" |
                        "ng-keydown" | "data-ng-keydown" |
                        "ng-keyup" | "data-ng-keyup" |
                        "ng-keypress" | "data-ng-keypress" |
                        "ng-mousedown" | "data-ng-mousedown" |
                        "ng-mouseup" | "data-ng-mouseup" |
                        "ng-mouseenter" | "data-ng-mouseenter" |
                        "ng-mouseleave" | "data-ng-mouseleave" |
                        "ng-mousemove" | "data-ng-mousemove" |
                        "ng-mouseover" | "data-ng-mouseover" |
                        "ng-copy" | "data-ng-copy" |
                        "ng-cut" | "data-ng-cut" |
                        "ng-paste" | "data-ng-paste" |
                        // セレクト
                        "ng-options" | "data-ng-options" |
                        // href/src
                        "ng-href" | "data-ng-href" |
                        "ng-src" | "data-ng-src" |
                        "ng-srcset" | "data-ng-srcset"
                    );

                    if is_ng_directive {
                        if let Some(value_node) = self.find_child_by_kind(child, "quoted_attribute_value") {
                            let raw_value = self.node_text(value_node, source);
                            let value = raw_value.trim_matches(|c| c == '"' || c == '\'');

                            // 式からプロパティ名を抽出
                            let property_paths = self.parse_angular_expression(value, &attr_name);
                            let line = value_node.start_position().row as u32;

                            // コントローラー名を解決（複数の継承コントローラーを含む）
                            let controller_names = self.index.resolve_controllers_for_html(uri, line);

                            // 属性値の開始位置（クォートの後）
                            let value_start_line = value_node.start_position().row as u32;
                            let value_start_col = value_node.start_position().column as u32 + 1; // +1 for quote

                            for property_path in property_paths {
                                // 属性値内で識別子のすべての出現位置を検索
                                let positions = self.find_identifier_positions(value, &property_path);

                                for (offset, len) in positions {
                                    let start_line = value_start_line;
                                    let start_col = value_start_col + offset as u32;
                                    let end_line = value_start_line;
                                    let end_col = start_col + len as u32;

                                    // HtmlScopeReferenceを登録
                                    let html_reference = HtmlScopeReference {
                                        property_path: property_path.clone(),
                                        uri: uri.clone(),
                                        start_line,
                                        start_col,
                                        end_line,
                                        end_col,
                                    };
                                    self.index.add_html_scope_reference(html_reference);

                                    // 全てのコントローラーに対してSymbolReferenceを登録
                                    for ctrl_name in &controller_names {
                                        let symbol_name = format!("{}.$scope.{}", ctrl_name, property_path);
                                        let symbol_reference = SymbolReference {
                                            name: symbol_name,
                                            uri: uri.clone(),
                                            start_line,
                                            start_col,
                                            end_line,
                                            end_col,
                                        };
                                        self.index.add_reference(symbol_reference);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// 文字列内で識別子のすべての出現位置を検索（単語境界を考慮）
    fn find_identifier_positions(&self, text: &str, identifier: &str) -> Vec<(usize, usize)> {
        let mut positions = Vec::new();
        let mut start = 0;

        while let Some(offset) = text[start..].find(identifier) {
            let abs_offset = start + offset;
            let end_offset = abs_offset + identifier.len();

            // 単語境界をチェック（識別子の前後が識別子文字でないこと）
            let before_ok = abs_offset == 0
                || !text[..abs_offset]
                    .chars()
                    .last()
                    .map(|c| c.is_alphanumeric() || c == '_' || c == '$')
                    .unwrap_or(false);

            let after_ok = end_offset >= text.len()
                || !text[end_offset..]
                    .chars()
                    .next()
                    .map(|c| c.is_alphanumeric() || c == '_' || c == '$')
                    .unwrap_or(false);

            if before_ok && after_ok {
                positions.push((abs_offset, identifier.len()));
            }

            start = abs_offset + 1;
        }

        positions
    }

    /// interpolation（デフォルト: {{...}}）からスコープ参照を抽出
    fn extract_interpolation_references(&self, text: &str, node: Node, _source: &str, uri: &Url) {
        let (start_symbol, end_symbol) = self.get_interpolate_symbols();
        let start_len = start_symbol.len();
        let end_len = end_symbol.len();

        let line = node.start_position().row as u32;
        // コントローラー名を解決（複数の継承コントローラーを含む）
        let controller_names = self.index.resolve_controllers_for_html(uri, line);

        let node_start_col = node.start_position().column as u32;

        let mut start = 0;
        while let Some(open_idx) = text[start..].find(&start_symbol) {
            let abs_open = start + open_idx;
            if let Some(close_idx) = text[abs_open..].find(&end_symbol) {
                let abs_close = abs_open + close_idx;
                let expr = &text[abs_open + start_len..abs_close];
                let expr_trimmed = expr.trim();

                // 式の開始位置（{{ の後、トリム前の空白を考慮）
                let expr_start_in_text = abs_open + start_len + (expr.len() - expr.trim_start().len());

                let property_paths = self.parse_angular_expression(expr_trimmed, "interpolation");

                for property_path in property_paths {
                    // 式内で識別子のすべての出現位置を検索
                    let positions = self.find_identifier_positions(expr_trimmed, &property_path);

                    for (offset, len) in positions {
                        let start_line = node.start_position().row as u32;
                        let start_col = node_start_col + expr_start_in_text as u32 + offset as u32;
                        let end_line = start_line;
                        let end_col = start_col + len as u32;

                        // HtmlScopeReferenceを登録
                        let html_reference = HtmlScopeReference {
                            property_path: property_path.clone(),
                            uri: uri.clone(),
                            start_line,
                            start_col,
                            end_line,
                            end_col,
                        };
                        self.index.add_html_scope_reference(html_reference);

                        // 全てのコントローラーに対してSymbolReferenceを登録
                        for ctrl_name in &controller_names {
                            let symbol_name = format!("{}.$scope.{}", ctrl_name, property_path);
                            let symbol_reference = SymbolReference {
                                name: symbol_name,
                                uri: uri.clone(),
                                start_line,
                                start_col,
                                end_line,
                                end_col,
                            };
                            self.index.add_reference(symbol_reference);
                        }
                    }
                }

                start = abs_close + end_len;
            } else {
                break;
            }
        }
    }

    /// AngularJS式からプロパティパスを抽出（tree-sitter使用）
    fn parse_angular_expression(&self, expr: &str, directive: &str) -> Vec<String> {
        let mut local_vars: Vec<String> = Vec::new();

        // ng-repeat: "item in items" or "(key, value) in items" -> ローカル変数を抽出
        let expr_to_parse = if directive.contains("ng-repeat") || directive.contains("data-ng-repeat") {
            if let Some(in_idx) = expr.find(" in ") {
                let iter_part = expr[..in_idx].trim();
                // (key, value) 形式
                if iter_part.starts_with('(') && iter_part.ends_with(')') {
                    let inner = &iter_part[1..iter_part.len()-1];
                    for var in inner.split(',') {
                        local_vars.push(var.trim().to_string());
                    }
                } else {
                    // item 形式
                    local_vars.push(iter_part.to_string());
                }
                // "in"の後の部分だけをパース
                &expr[in_idx + 4..]
            } else {
                expr
            }
        } else {
            expr
        };

        // フィルター部分を除去（AngularJSフィルターはJS構文ではない）
        let expr_to_parse = expr_to_parse.split('|').next().unwrap_or(expr_to_parse).trim();

        // tree-sitter-javascriptで式をパース
        let mut parser = JsParser::new();
        let mut identifiers = Vec::new();

        if let Some(tree) = parser.parse(expr_to_parse) {
            self.collect_identifiers_from_expr(tree.root_node(), expr_to_parse, &mut identifiers);
        }

        // ローカル変数とAngularキーワードを除外
        identifiers
            .into_iter()
            .filter(|name| !local_vars.contains(name) && !self.is_angular_keyword(name))
            .collect()
    }

    /// 式のASTから識別子を収集
    fn collect_identifiers_from_expr(&self, node: tree_sitter::Node, source: &str, identifiers: &mut Vec<String>) {
        match node.kind() {
            // member_expression: user.name -> "user"のみ抽出
            "member_expression" => {
                if let Some(object) = node.child_by_field_name("object") {
                    // ネストしたmember_expression (a.b.c) の場合は再帰
                    if object.kind() == "member_expression" {
                        self.collect_identifiers_from_expr(object, source, identifiers);
                    } else if object.kind() == "identifier" {
                        let name = self.node_text(object, source);
                        if !identifiers.contains(&name) {
                            identifiers.push(name);
                        }
                    } else {
                        // call_expression等の場合は子を探索
                        self.collect_identifiers_from_expr(object, source, identifiers);
                    }
                }
                // argumentsがある場合（メソッド呼び出しの引数など）
                // member_expressionの子ノードも探索
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() != "identifier" && child.kind() != "property_identifier" {
                        self.collect_identifiers_from_expr(child, source, identifiers);
                    }
                }
            }
            // call_expression: save(user) -> "save"と"user"を抽出
            "call_expression" => {
                if let Some(func) = node.child_by_field_name("function") {
                    self.collect_identifiers_from_expr(func, source, identifiers);
                }
                if let Some(args) = node.child_by_field_name("arguments") {
                    self.collect_identifiers_from_expr(args, source, identifiers);
                }
            }
            // 単独の識別子
            "identifier" => {
                let name = self.node_text(node, source);
                if !identifiers.contains(&name) {
                    identifiers.push(name);
                }
            }
            // その他のノードは子を再帰的に探索
            _ => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    self.collect_identifiers_from_expr(child, source, identifiers);
                }
            }
        }
    }

    /// AngularJSのキーワードかどうか
    fn is_angular_keyword(&self, name: &str) -> bool {
        matches!(
            name,
            "true" | "false" | "null" | "undefined" |
            "$index" | "$first" | "$last" | "$middle" | "$odd" | "$even" |
            "track" | "by" | "in" | "as" |
            // ng-repeatでよく使われるローカル変数名
            "item" | "key" | "value" | "i" | "idx" |
            // JavaScript組み込み
            "console" | "window" | "document" | "Math" | "JSON" | "Array" | "Object" | "String" | "Number"
        )
    }

    /// カーソル位置がAngularディレクティブまたはinterpolation内にあるかを判定
    /// 戻り値: true = Angular コンテキスト内（$scope補完が必要）
    pub fn is_in_angular_context(&self, source: &str, line: u32, col: u32) -> bool {
        let lines: Vec<&str> = source.lines().collect();
        if (line as usize) >= lines.len() {
            return false;
        }

        let current_line = lines[line as usize];
        let col = col as usize;
        if col > current_line.len() {
            return false;
        }

        let before_cursor = &current_line[..col];

        // 1. interpolation内かチェック（{{ ... }}）
        let (start_symbol, end_symbol) = self.get_interpolate_symbols();
        if let Some(open_idx) = before_cursor.rfind(&start_symbol) {
            // 開き記号の後に閉じ記号がないかチェック
            let after_open = &before_cursor[open_idx + start_symbol.len()..];
            if !after_open.contains(&end_symbol) {
                return true;
            }
        }

        // 2. AngularJSディレクティブ属性内かチェック
        // ng-if="...", ng-model="...", ng-click="..." など
        let ng_directives = [
            // データバインディング
            "ng-model", "data-ng-model",
            "ng-bind", "data-ng-bind",
            "ng-bind-html", "data-ng-bind-html",
            "ng-value", "data-ng-value",
            "ng-init", "data-ng-init",
            // 条件・繰り返し
            "ng-if", "data-ng-if",
            "ng-show", "data-ng-show",
            "ng-hide", "data-ng-hide",
            "ng-repeat", "data-ng-repeat",
            "ng-switch", "data-ng-switch",
            "ng-switch-when", "data-ng-switch-when",
            // スタイル・クラス
            "ng-class", "data-ng-class",
            "ng-style", "data-ng-style",
            // フォームバリデーション
            "ng-disabled", "data-ng-disabled",
            "ng-checked", "data-ng-checked",
            "ng-selected", "data-ng-selected",
            "ng-readonly", "data-ng-readonly",
            "ng-required", "data-ng-required",
            "ng-pattern", "data-ng-pattern",
            "ng-minlength", "data-ng-minlength",
            "ng-maxlength", "data-ng-maxlength",
            // イベントハンドラ
            "ng-click", "data-ng-click",
            "ng-dblclick", "data-ng-dblclick",
            "ng-change", "data-ng-change",
            "ng-submit", "data-ng-submit",
            "ng-blur", "data-ng-blur",
            "ng-focus", "data-ng-focus",
            "ng-keydown", "data-ng-keydown",
            "ng-keyup", "data-ng-keyup",
            "ng-keypress", "data-ng-keypress",
            "ng-mousedown", "data-ng-mousedown",
            "ng-mouseup", "data-ng-mouseup",
            "ng-mouseenter", "data-ng-mouseenter",
            "ng-mouseleave", "data-ng-mouseleave",
            "ng-mousemove", "data-ng-mousemove",
            "ng-mouseover", "data-ng-mouseover",
            "ng-copy", "data-ng-copy",
            "ng-cut", "data-ng-cut",
            "ng-paste", "data-ng-paste",
            // セレクト
            "ng-options", "data-ng-options",
            // href/src
            "ng-href", "data-ng-href",
            "ng-src", "data-ng-src",
            "ng-srcset", "data-ng-srcset",
        ];

        for directive in &ng_directives {
            // ng-if="..." パターンを検索
            let pattern = format!("{}=\"", directive);
            if let Some(attr_start) = before_cursor.rfind(&pattern) {
                let after_attr = &before_cursor[attr_start + pattern.len()..];
                // 属性値の閉じクォートがないかチェック
                if !after_attr.contains('"') {
                    return true;
                }
            }
            // シングルクォートパターンもチェック
            let pattern_single = format!("{}='", directive);
            if let Some(attr_start) = before_cursor.rfind(&pattern_single) {
                let after_attr = &before_cursor[attr_start + pattern_single.len()..];
                if !after_attr.contains('\'') {
                    return true;
                }
            }
        }

        false
    }

    /// 指定した種類の子ノードを検索
    fn find_child_by_kind<'a>(&self, node: Node<'a>, kind: &str) -> Option<Node<'a>> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == kind {
                return Some(child);
            }
        }
        None
    }

    /// ノードのテキストを取得
    fn node_text(&self, node: Node, source: &str) -> String {
        source[node.byte_range()].to_string()
    }

    /// 文字列ノードから値を取得（クォートを除去）
    fn extract_string_value(&self, node: Node, source: &str) -> String {
        let text = self.node_text(node, source);
        text.trim_matches(|c| c == '"' || c == '\'').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::SymbolIndex;

    fn create_analyzer() -> (HtmlAngularJsAnalyzer, Arc<SymbolIndex>) {
        let index = Arc::new(SymbolIndex::new());
        let js_analyzer = Arc::new(AngularJsAnalyzer::new(Arc::clone(&index)));
        let analyzer = HtmlAngularJsAnalyzer::new(Arc::clone(&index), js_analyzer);
        (analyzer, index)
    }

    #[test]
    fn test_ng_controller_scope_detection() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<div ng-controller="UserController">
    <span>Hello</span>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // ng-controllerスコープが検出されているか
        let controller = index.get_html_controller_at(&uri, 1);
        assert_eq!(controller, Some("UserController".to_string()));
    }

    #[test]
    fn test_nested_ng_controller() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<div ng-controller="OuterController">
    <div ng-controller="InnerController">
        <span>Inner</span>
    </div>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // 外側のコントローラー
        let outer = index.get_html_controller_at(&uri, 1);
        assert_eq!(outer, Some("OuterController".to_string()));

        // 内側のコントローラー（より狭いスコープを優先）
        let inner = index.get_html_controller_at(&uri, 3);
        assert_eq!(inner, Some("InnerController".to_string()));
    }

    #[test]
    fn test_ng_model_reference() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<div ng-controller="UserController">
    <input ng-model="user.name">
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // ng-modelからスコープ参照が抽出されているか
        // "user" starts at column 21 in `    <input ng-model="user.name">`
        let ref_opt = index.find_html_scope_reference_at(&uri, 2, 21);
        assert!(ref_opt.is_some(), "ng-model reference should be found");
        assert_eq!(ref_opt.unwrap().property_path, "user");
    }

    #[test]
    fn test_ng_click_reference() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<div ng-controller="UserController">
    <button ng-click="save()">Save</button>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // ng-clickからスコープ参照が抽出されているか
        let ref_opt = index.find_html_scope_reference_at(&uri, 2, 22);
        assert!(ref_opt.is_some(), "ng-click reference should be found");
        assert_eq!(ref_opt.unwrap().property_path, "save");
    }

    #[test]
    fn test_ng_repeat_reference() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<div ng-controller="UserController">
    <div ng-repeat="item in items">{{item.name}}</div>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // ng-repeatからコレクション参照が抽出されているか
        // "items" starts at column 28 in `    <div ng-repeat="item in items">`
        let ref_opt = index.find_html_scope_reference_at(&uri, 2, 28);
        assert!(ref_opt.is_some(), "ng-repeat reference should be found");
        assert_eq!(ref_opt.unwrap().property_path, "items");
    }

    #[test]
    fn test_interpolation_reference() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<div ng-controller="UserController">
    <span>{{message}}</span>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // {{interpolation}}からスコープ参照が抽出されているか
        let ref_opt = index.find_html_scope_reference_at(&uri, 2, 12);
        assert!(ref_opt.is_some(), "interpolation reference should be found");
        assert_eq!(ref_opt.unwrap().property_path, "message");
    }

    #[test]
    fn test_ng_if_reference() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<div ng-controller="UserController">
    <span ng-if="isVisible">Hello</span>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        let ref_opt = index.find_html_scope_reference_at(&uri, 2, 18);
        assert!(ref_opt.is_some(), "ng-if reference should be found");
        assert_eq!(ref_opt.unwrap().property_path, "isVisible");
    }

    #[test]
    fn test_ng_show_reference() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<div ng-controller="UserController">
    <span ng-show="showMessage">Hello</span>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        let ref_opt = index.find_html_scope_reference_at(&uri, 2, 20);
        assert!(ref_opt.is_some(), "ng-show reference should be found");
        assert_eq!(ref_opt.unwrap().property_path, "showMessage");
    }

    #[test]
    fn test_script_tag_route_binding() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<script>
angular.module('app').config(function($routeProvider) {
    $routeProvider.when('/users', {
        controller: 'UserController',
        templateUrl: 'views/users.html'
    });
});
</script>
"#;
        analyzer.analyze_document(&uri, html);

        // テンプレートバインディングが抽出されているか
        let controller = index.get_controller_for_template(
            &Url::parse("file:///views/users.html").unwrap()
        );
        assert_eq!(controller, Some("UserController".to_string()));
    }

    #[test]
    fn test_script_tag_modal_binding() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<script>
$uibModal.open({
    controller: 'ModalController',
    templateUrl: 'views/modal.html'
});
</script>
"#;
        analyzer.analyze_document(&uri, html);

        // モーダルバインディングが抽出されているか
        let controller = index.get_controller_for_template(
            &Url::parse("file:///views/modal.html").unwrap()
        );
        assert_eq!(controller, Some("ModalController".to_string()));
    }

    #[test]
    fn test_data_ng_controller() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<div data-ng-controller="UserController">
    <span>Hello</span>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // data-ng-controllerも認識されるか
        let controller = index.get_html_controller_at(&uri, 1);
        assert_eq!(controller, Some("UserController".to_string()));
    }

    #[test]
    fn test_complex_expression() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<div ng-controller="UserController">
    <span ng-if="isActive && isEnabled">Active</span>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // 複数のプロパティが抽出されているか（最初の一つをテスト）
        let ref_opt = index.find_html_scope_reference_at(&uri, 2, 18);
        assert!(ref_opt.is_some(), "complex expression reference should be found");
    }

    #[test]
    fn test_filter_expression() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<div ng-controller="UserController">
    <span>{{amount | currency}}</span>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // フィルター付き式からプロパティが抽出されているか
        let ref_opt = index.find_html_scope_reference_at(&uri, 2, 13);
        assert!(ref_opt.is_some(), "filter expression reference should be found");
        assert_eq!(ref_opt.unwrap().property_path, "amount");
    }

    #[test]
    fn test_resolve_controller_for_html() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<div ng-controller="UserController">
    <input ng-model="userName">
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // resolve_controller_for_htmlが正しく動作するか
        let controller = index.resolve_controller_for_html(&uri, 2);
        assert_eq!(controller, Some("UserController".to_string()));
    }

    #[test]
    fn test_template_binding_resolution() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///app.html").unwrap();

        let html = r#"
<script>
$routeProvider.when('/profile', {
    controller: 'ProfileController',
    templateUrl: 'views/profile.html?v=123'
});
</script>
"#;
        analyzer.analyze_document(&uri, html);

        // クエリパラメータ付きテンプレートパスが正しく解決されるか
        let template_uri = Url::parse("file:///views/profile.html").unwrap();
        let controller = index.resolve_controller_for_html(&template_uri, 0);
        assert_eq!(controller, Some("ProfileController".to_string()));
    }

    #[test]
    fn test_method_call_with_arguments() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<div ng-controller="UserController">
    <button ng-click="vm.save(user.id)">Save</button>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // vm.save(user.id) から vm と user の両方が抽出されているか
        let symbols = index.get_document_symbols(&uri);
        let scope_props: Vec<_> = symbols.iter().filter(|s| s.kind == crate::index::SymbolKind::ScopeProperty).collect();

        let names: Vec<_> = scope_props.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"vm"), "vm should be extracted from ng-click");
        assert!(names.contains(&"user"), "user should be extracted from ng-click arguments");
    }

    #[test]
    fn test_ng_repeat_key_value() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<div ng-controller="UserController">
    <div ng-repeat="(key, value) in items">{{key}}: {{value}}</div>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // (key, value) in items から items のみが抽出され、key/value は除外されているか
        let symbols = index.get_document_symbols(&uri);
        let scope_props: Vec<_> = symbols.iter().filter(|s| s.kind == crate::index::SymbolKind::ScopeProperty).collect();

        let names: Vec<_> = scope_props.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"items"), "items should be extracted from ng-repeat");
        // key, valueはローカル変数なので除外
        assert!(!names.iter().any(|n| *n == "key"), "key should NOT be extracted (local var)");
        assert!(!names.iter().any(|n| *n == "value"), "value should NOT be extracted (local var)");
    }

    #[test]
    fn test_nested_member_expression() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<div ng-controller="UserController">
    <span ng-if="vm.user.isActive">Active</span>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // vm.user.isActive から vm のみが抽出されているか
        let symbols = index.get_document_symbols(&uri);
        let scope_props: Vec<_> = symbols.iter().filter(|s| s.kind == crate::index::SymbolKind::ScopeProperty).collect();

        let names: Vec<_> = scope_props.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"vm"), "vm should be extracted from nested member expression");
    }

    #[test]
    fn test_ternary_expression() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<div ng-controller="UserController">
    <span>{{isActive ? activeLabel : inactiveLabel}}</span>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // 三項演算子から全ての識別子が抽出されているか
        let symbols = index.get_document_symbols(&uri);
        let scope_props: Vec<_> = symbols.iter().filter(|s| s.kind == crate::index::SymbolKind::ScopeProperty).collect();

        let names: Vec<_> = scope_props.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"isActive"), "isActive should be extracted");
        assert!(names.contains(&"activeLabel"), "activeLabel should be extracted");
        assert!(names.contains(&"inactiveLabel"), "inactiveLabel should be extracted");
    }

    #[test]
    fn test_custom_interpolate_symbols() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        // カスタムinterpolate記号を設定
        analyzer.set_interpolate_config(crate::config::InterpolateConfig {
            start_symbol: "[[".to_string(),
            end_symbol: "]]".to_string(),
        });

        let html = r#"
<div ng-controller="UserController">
    <span>[[message]]</span>
    <span>{{ignored}}</span>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // [[ ... ]] からは抽出されるが、{{ ... }} からは抽出されない
        let symbols = index.get_document_symbols(&uri);
        let scope_props: Vec<_> = symbols.iter().filter(|s| s.kind == crate::index::SymbolKind::ScopeProperty).collect();

        let names: Vec<_> = scope_props.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"message"), "message should be extracted from [[...]]");
        assert!(!names.contains(&"ignored"), "ignored should NOT be extracted from {{...}}");
    }

    #[test]
    fn test_custom_interpolate_with_expressions() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        // カスタムinterpolate記号を設定（ERBスタイルは避け、より一般的な記号を使用）
        analyzer.set_interpolate_config(crate::config::InterpolateConfig {
            start_symbol: "[[".to_string(),
            end_symbol: "]]".to_string(),
        });

        let html = r#"
<div ng-controller="UserController">
    <span>[[ user.name ]]</span>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        let symbols = index.get_document_symbols(&uri);
        let scope_props: Vec<_> = symbols.iter().filter(|s| s.kind == crate::index::SymbolKind::ScopeProperty).collect();

        let names: Vec<_> = scope_props.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"user"), "user should be extracted from [[...]]");
    }

    #[test]
    fn test_html_scope_reference_registered_as_symbol_reference() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<div ng-controller="UserController">
    <input ng-model="userName">
    <span>{{userMessage}}</span>
    <button ng-click="save()">Save</button>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // SymbolReferenceとして登録されているか確認
        let user_name_refs = index.get_references("UserController.$scope.userName");
        assert!(!user_name_refs.is_empty(), "userName should be registered as SymbolReference");

        let user_message_refs = index.get_references("UserController.$scope.userMessage");
        assert!(!user_message_refs.is_empty(), "userMessage should be registered as SymbolReference");

        let save_refs = index.get_references("UserController.$scope.save");
        assert!(!save_refs.is_empty(), "save should be registered as SymbolReference");
    }

    #[test]
    fn test_html_scope_reference_with_template_binding() {
        let (analyzer, index) = create_analyzer();
        let app_uri = Url::parse("file:///app.html").unwrap();

        // まずテンプレートバインディングを設定
        let app_html = r#"
<script>
$routeProvider.when('/users', {
    controller: 'UserController',
    templateUrl: 'views/users.html'
});
</script>
"#;
        analyzer.analyze_document(&app_uri, app_html);

        // テンプレートファイルを解析
        let template_uri = Url::parse("file:///views/users.html").unwrap();
        let template_html = r#"
<div>
    <span>{{userName}}</span>
</div>
"#;
        analyzer.analyze_document(&template_uri, template_html);

        // テンプレートバインディング経由でコントローラー名が解決され、SymbolReferenceが登録されているか
        let refs = index.get_references("UserController.$scope.userName");
        assert!(!refs.is_empty(), "userName should be registered via template binding");
        assert_eq!(refs[0].uri, template_uri);
    }

    #[test]
    fn test_html_scope_reference_in_ng_if() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<div ng-controller="AppController">
    <span ng-if="isVisible && isEnabled">Content</span>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // ng-if内の複数の識別子がSymbolReferenceとして登録されているか
        let is_visible_refs = index.get_references("AppController.$scope.isVisible");
        assert!(!is_visible_refs.is_empty(), "isVisible should be registered as SymbolReference");

        let is_enabled_refs = index.get_references("AppController.$scope.isEnabled");
        assert!(!is_enabled_refs.is_empty(), "isEnabled should be registered as SymbolReference");
    }

    #[test]
    fn test_is_in_angular_context_ng_if() {
        let (analyzer, _index) = create_analyzer();

        let html = r#"<div ng-controller="UserController">
    <span ng-if="isVisible">Content</span>
</div>"#;

        // ng-if="isVisible" 内にカーソルがある場合
        // 行1、列17 は ng-if=" の直後
        assert!(analyzer.is_in_angular_context(html, 1, 17), "Should be in Angular context (ng-if start)");

        // 行1、列25 は isVisible の途中
        assert!(analyzer.is_in_angular_context(html, 1, 22), "Should be in Angular context (ng-if middle)");

        // 行0、列5 は ng-controller 属性外
        assert!(!analyzer.is_in_angular_context(html, 0, 5), "Should NOT be in Angular context (outside)");
    }

    #[test]
    fn test_is_in_angular_context_interpolation() {
        let (analyzer, _index) = create_analyzer();

        let html = r#"<div ng-controller="UserController">
    <span>{{message}}</span>
</div>"#;

        // {{ の直後
        assert!(analyzer.is_in_angular_context(html, 1, 12), "Should be in Angular context (interpolation start)");

        // message の途中
        assert!(analyzer.is_in_angular_context(html, 1, 15), "Should be in Angular context (interpolation middle)");

        // }} の外
        assert!(!analyzer.is_in_angular_context(html, 1, 5), "Should NOT be in Angular context (outside interpolation)");
    }

    #[test]
    fn test_is_in_angular_context_ng_model() {
        let (analyzer, _index) = create_analyzer();

        let html = r#"<input ng-model="userName">"#;

        // ng-model=" の直後
        assert!(analyzer.is_in_angular_context(html, 0, 17), "Should be in Angular context (ng-model)");

        // userName の途中
        assert!(analyzer.is_in_angular_context(html, 0, 20), "Should be in Angular context (ng-model middle)");
    }

    #[test]
    fn test_is_in_angular_context_ng_click() {
        let (analyzer, _index) = create_analyzer();

        let html = r#"<button ng-click="save()">Save</button>"#;

        // ng-click=" の直後
        assert!(analyzer.is_in_angular_context(html, 0, 18), "Should be in Angular context (ng-click)");
    }

    #[test]
    fn test_is_in_angular_context_custom_interpolate() {
        let (analyzer, _index) = create_analyzer();

        // カスタムinterpolate記号を設定
        analyzer.set_interpolate_config(crate::config::InterpolateConfig {
            start_symbol: "[[".to_string(),
            end_symbol: "]]".to_string(),
        });

        let html = r#"<span>[[message]]</span>"#;

        // [[ の直後
        assert!(analyzer.is_in_angular_context(html, 0, 8), "Should be in Angular context (custom interpolation)");

        // デフォルトの {{ は認識されない
        let html_default = r#"<span>{{message}}</span>"#;
        assert!(!analyzer.is_in_angular_context(html_default, 0, 8), "Should NOT be in Angular context (wrong symbols)");
    }

    #[test]
    fn test_ng_include_attribute_detection() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///parent.html").unwrap();

        let html = r#"
<div ng-controller="ParentController">
    <div ng-controller="ChildController">
        <div ng-include="'child.html'"></div>
    </div>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // ng-includeで継承されるコントローラーを確認
        let child_uri = Url::parse("file:///child.html").unwrap();
        let inherited = index.get_inherited_controllers_for_template(&child_uri);
        assert_eq!(inherited.len(), 2, "Should inherit 2 controllers");
        assert_eq!(inherited[0], "ParentController");
        assert_eq!(inherited[1], "ChildController");
    }

    #[test]
    fn test_ng_include_element_detection() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///parent.html").unwrap();

        let html = r#"
<div ng-controller="MainController">
    <ng-include src="'partial.html'"></ng-include>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // ng-include要素で継承されるコントローラーを確認
        let child_uri = Url::parse("file:///partial.html").unwrap();
        let inherited = index.get_inherited_controllers_for_template(&child_uri);
        assert_eq!(inherited.len(), 1, "Should inherit 1 controller");
        assert_eq!(inherited[0], "MainController");
    }

    #[test]
    fn test_child_html_multiple_controller_references() {
        let (analyzer, index) = create_analyzer();
        let parent_uri = Url::parse("file:///parent.html").unwrap();

        // 親HTMLでng-includeを定義
        let parent_html = r#"
<div ng-controller="ParentController">
    <div ng-controller="ChildController">
        <div ng-include="'child.html'"></div>
    </div>
</div>
"#;
        analyzer.analyze_document(&parent_uri, parent_html);

        // 子HTMLを解析
        let child_uri = Url::parse("file:///child.html").unwrap();
        let child_html = r#"
<div>
    <span>{{message}}</span>
</div>
"#;
        analyzer.analyze_document(&child_uri, child_html);

        // 子HTMLの参照が両方のコントローラーに対して登録されているか
        let parent_refs = index.get_references("ParentController.$scope.message");
        let child_refs = index.get_references("ChildController.$scope.message");

        assert!(!parent_refs.is_empty(), "message should be registered for ParentController");
        assert!(!child_refs.is_empty(), "message should be registered for ChildController");
    }

    #[test]
    fn test_resolve_controllers_for_html_with_inheritance() {
        let (analyzer, index) = create_analyzer();
        let parent_uri = Url::parse("file:///parent.html").unwrap();

        // 親HTMLでng-includeを定義
        let parent_html = r#"
<div ng-controller="OuterController">
    <div ng-controller="InnerController">
        <div ng-include="'included.html'"></div>
    </div>
</div>
"#;
        analyzer.analyze_document(&parent_uri, parent_html);

        // 子HTMLで追加のng-controllerがある場合
        let child_uri = Url::parse("file:///included.html").unwrap();
        let child_html = r#"
<div ng-controller="LocalController">
    <span>{{value}}</span>
</div>
"#;
        analyzer.analyze_document(&child_uri, child_html);

        // resolve_controllers_for_htmlが全てのコントローラーを返すか
        let controllers = index.resolve_controllers_for_html(&child_uri, 2);
        assert!(controllers.contains(&"OuterController".to_string()), "Should contain OuterController");
        assert!(controllers.contains(&"InnerController".to_string()), "Should contain InnerController");
        assert!(controllers.contains(&"LocalController".to_string()), "Should contain LocalController");
    }

    #[test]
    fn test_data_ng_include_attribute() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///parent.html").unwrap();

        let html = r#"
<div ng-controller="TestController">
    <div data-ng-include="'template.html'"></div>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // data-ng-includeも検出されるか
        let child_uri = Url::parse("file:///template.html").unwrap();
        let inherited = index.get_inherited_controllers_for_template(&child_uri);
        assert_eq!(inherited.len(), 1, "Should inherit 1 controller");
        assert_eq!(inherited[0], "TestController");
    }

    #[test]
    fn test_get_html_controllers_at_order() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        let html = r#"
<div ng-controller="FirstController">
    <div ng-controller="SecondController">
        <div ng-controller="ThirdController">
            <span>Content</span>
        </div>
    </div>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // 全てのコントローラーが外側から内側の順で取得されるか
        let controllers = index.get_html_controllers_at(&uri, 4);
        assert_eq!(controllers.len(), 3);
        assert_eq!(controllers[0], "FirstController");
        assert_eq!(controllers[1], "SecondController");
        assert_eq!(controllers[2], "ThirdController");
    }

    #[test]
    fn test_ng_if_outside_controller_scope() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///test.html").unwrap();

        // ng-ifがaControllerの外側にある場合
        let html = r#"<div ng-if="status">
    <div ng-controller="aController">
        <span>{{innerValue}}</span>
    </div>
</div>"#;
        analyzer.analyze_document(&uri, html);

        // statusはaControllerの外側（行0）にある
        // aControllerは行1から始まる
        // statusはaControllerのスコープに含まれてはいけない
        let status_refs = index.get_references("aController.$scope.status");
        assert!(status_refs.is_empty(), "status should NOT be registered for aController (it's outside the controller scope)");

        // innerValueはaControllerの内側（行2）にある
        let inner_refs = index.get_references("aController.$scope.innerValue");
        assert!(!inner_refs.is_empty(), "innerValue should be registered for aController");
    }

    #[test]
    fn test_ng_include_with_dynamic_path() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///parent.html").unwrap();

        // 動的なパス（文字列連結）を含むng-include
        let html = r#"
<div ng-controller="MainController">
    <div ng-include="'../static/wf/views/request_expense/request_expense_view.html?' + app_version"></div>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // 文字列リテラル部分が抽出されているか
        let child_uri = Url::parse("file:///request_expense_view.html").unwrap();
        let inherited = index.get_inherited_controllers_for_template(&child_uri);
        assert_eq!(inherited.len(), 1, "Should inherit 1 controller even with dynamic path");
        assert_eq!(inherited[0], "MainController");
    }

    #[test]
    fn test_ng_include_with_query_param_and_version() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///parent.html").unwrap();

        // クエリパラメータ付きのパス
        let html = r#"
<div ng-controller="TestController">
    <div ng-include="'views/modal.html?v=' + version"></div>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // ファイル名部分でマッチするか（クエリパラメータは除去される）
        let child_uri = Url::parse("file:///modal.html").unwrap();
        let inherited = index.get_inherited_controllers_for_template(&child_uri);
        assert_eq!(inherited.len(), 1, "Should inherit 1 controller");
        assert_eq!(inherited[0], "TestController");
    }

    #[test]
    fn test_ng_include_with_relative_path() {
        let (analyzer, index) = create_analyzer();
        // 親ファイルが /app/views/main.html にある場合
        let uri = Url::parse("file:///app/views/main.html").unwrap();

        // 相対パス ../static/wf/views/request/request_details.html
        let html = r#"
<div ng-controller="MainController">
    <div ng-include="'../static/wf/views/request/request_details.html?' + app_version"></div>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // 親ファイル /app/views/main.html を基準に解決すると
        // /app/static/wf/views/request/request_details.html になる
        // ファイル名は request_details.html
        let child_uri = Url::parse("file:///app/static/wf/views/request/request_details.html").unwrap();
        let inherited = index.get_inherited_controllers_for_template(&child_uri);
        assert_eq!(inherited.len(), 1, "Should inherit 1 controller via relative path resolution");
        assert_eq!(inherited[0], "MainController");
    }

    #[test]
    fn test_ng_include_with_absolute_path() {
        let (analyzer, index) = create_analyzer();
        let uri = Url::parse("file:///app/views/main.html").unwrap();

        // 絶対パス /static/templates/header.html
        let html = r#"
<div ng-controller="HeaderController">
    <div ng-include="'/static/templates/header.html'"></div>
</div>
"#;
        analyzer.analyze_document(&uri, html);

        // 絶対パスの場合はそのまま解決
        let child_uri = Url::parse("file:///static/templates/header.html").unwrap();
        let inherited = index.get_inherited_controllers_for_template(&child_uri);
        assert_eq!(inherited.len(), 1, "Should inherit 1 controller via absolute path");
        assert_eq!(inherited[0], "HeaderController");
    }

    #[test]
    fn test_resolve_relative_path_function() {
        use crate::index::SymbolIndex;

        // 基本的な相対パス解決
        let parent_uri = Url::parse("file:///app/views/main.html").unwrap();

        // ../を含むパス
        let result = SymbolIndex::resolve_relative_path(&parent_uri, "../static/test.html");
        assert_eq!(result, "test.html");

        // 複数の../を含むパス
        let result = SymbolIndex::resolve_relative_path(&parent_uri, "../../templates/modal.html");
        assert_eq!(result, "modal.html");

        // 単純な相対パス
        let result = SymbolIndex::resolve_relative_path(&parent_uri, "partials/header.html");
        assert_eq!(result, "header.html");

        // 絶対パス
        let result = SymbolIndex::resolve_relative_path(&parent_uri, "/static/footer.html");
        assert_eq!(result, "footer.html");

        // クエリパラメータ付き
        let result = SymbolIndex::resolve_relative_path(&parent_uri, "../views/detail.html?v=123");
        assert_eq!(result, "detail.html");
    }
}
