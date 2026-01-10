//! ng-controllerスコープとng-includeの収集

use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use crate::index::{
    HtmlControllerScope, HtmlFormBinding, HtmlLocalVariableSource, InheritedFormBinding,
    InheritedLocalVariable, NgIncludeBinding, SymbolIndex,
};

use super::HtmlAngularJsAnalyzer;

/// ローカル変数スコープ情報（収集時に使用）
#[derive(Clone, Debug)]
struct LocalVariableScope {
    name: String,
    source: HtmlLocalVariableSource,
    scope_start_line: u32,
    scope_end_line: u32,
    name_start_line: u32,
    name_start_col: u32,
    name_end_line: u32,
    name_end_col: u32,
}

/// フォームバインディングスコープ情報（収集時に使用）
#[derive(Clone, Debug)]
struct FormBindingScope {
    name: String,
    scope_start_line: u32,
    scope_end_line: u32,
    name_start_line: u32,
    name_start_col: u32,
    name_end_line: u32,
    name_end_col: u32,
}

impl HtmlAngularJsAnalyzer {
    /// ng-controllerスコープとng-includeを収集
    pub(super) fn collect_controller_scopes_and_includes(
        &self,
        node: Node,
        source: &str,
        uri: &Url,
        controller_stack: &mut Vec<String>,
    ) {
        // ローカル変数スタックを追加
        let mut local_var_stack: Vec<LocalVariableScope> = Vec::new();
        // フォームバインディングスタックを追加
        let mut form_binding_stack: Vec<FormBindingScope> = Vec::new();
        self.collect_controller_scopes_and_includes_with_locals(
            node,
            source,
            uri,
            controller_stack,
            &mut local_var_stack,
            &mut form_binding_stack,
        );
    }

    /// ng-controllerスコープ、ng-include、ローカル変数、フォームバインディングを収集
    fn collect_controller_scopes_and_includes_with_locals(
        &self,
        node: Node,
        source: &str,
        uri: &Url,
        controller_stack: &mut Vec<String>,
        local_var_stack: &mut Vec<LocalVariableScope>,
        form_binding_stack: &mut Vec<FormBindingScope>,
    ) {
        // 要素ノードの場合、ng-controller属性をチェック
        if node.kind() == "element" {
            let mut added_controller = false;
            let mut added_local_vars: Vec<String> = Vec::new();
            let mut added_form_binding: Option<String> = None;

            let scope_start_line = node.start_position().row as u32;
            let scope_end_line = node.end_position().row as u32;

            // 開始タグから属性を取得
            if let Some(start_tag) = self.find_child_by_kind(node, "start_tag") {
                // ng-controllerをチェック
                if let Some((controller_name, alias)) =
                    self.get_ng_controller_attribute(start_tag, source)
                {
                    // ng-controllerスコープを登録
                    let scope = HtmlControllerScope {
                        controller_name: controller_name.clone(),
                        alias,
                        uri: uri.clone(),
                        start_line: scope_start_line,
                        end_line: scope_end_line,
                    };
                    self.index.add_html_controller_scope(scope);
                    controller_stack.push(controller_name);
                    added_controller = true;
                }

                // ng-repeatからローカル変数を抽出
                if let Some(vars) =
                    self.extract_local_vars_from_ng_repeat(start_tag, source, scope_start_line, scope_end_line)
                {
                    for var in vars {
                        added_local_vars.push(var.name.clone());
                        local_var_stack.push(var);
                    }
                }

                // ng-initからローカル変数を抽出
                if let Some(vars) =
                    self.extract_local_vars_from_ng_init(start_tag, source, scope_start_line, scope_end_line)
                {
                    for var in vars {
                        added_local_vars.push(var.name.clone());
                        local_var_stack.push(var);
                    }
                }

                // <form name="x">からフォームバインディングを抽出
                if let Some(form_scope) =
                    self.extract_form_name_from_tag(start_tag, source, scope_start_line, scope_end_line)
                {
                    // HtmlFormBindingとして登録
                    let binding = HtmlFormBinding {
                        name: form_scope.name.clone(),
                        uri: uri.clone(),
                        scope_start_line: form_scope.scope_start_line,
                        scope_end_line: form_scope.scope_end_line,
                        name_start_line: form_scope.name_start_line,
                        name_start_col: form_scope.name_start_col,
                        name_end_line: form_scope.name_end_line,
                        name_end_col: form_scope.name_end_col,
                    };
                    self.index.add_html_form_binding(binding);
                    added_form_binding = Some(form_scope.name.clone());
                    form_binding_stack.push(form_scope);
                }

                // ng-includeをチェック
                if let Some(template_path) = self.get_ng_include_attribute(start_tag, source) {
                    // 親ファイルを起点として相対パスを解決
                    let resolved_filename = SymbolIndex::resolve_relative_path(uri, &template_path);

                    // 現在のローカル変数スタックをInheritedLocalVariableに変換
                    let inherited_local_variables: Vec<InheritedLocalVariable> = local_var_stack
                        .iter()
                        .map(|v| InheritedLocalVariable {
                            name: v.name.clone(),
                            source: v.source.clone(),
                            uri: uri.clone(),
                            scope_start_line: v.scope_start_line,
                            scope_end_line: v.scope_end_line,
                            name_start_line: v.name_start_line,
                            name_start_col: v.name_start_col,
                            name_end_line: v.name_end_line,
                            name_end_col: v.name_end_col,
                        })
                        .collect();

                    // 現在のフォームバインディングスタックをInheritedFormBindingに変換
                    let inherited_form_bindings: Vec<InheritedFormBinding> = form_binding_stack
                        .iter()
                        .map(|f| InheritedFormBinding {
                            name: f.name.clone(),
                            uri: uri.clone(),
                            scope_start_line: f.scope_start_line,
                            scope_end_line: f.scope_end_line,
                            name_start_line: f.name_start_line,
                            name_start_col: f.name_start_col,
                            name_end_line: f.name_end_line,
                            name_end_col: f.name_end_col,
                        })
                        .collect();

                    // 現在のコントローラースタックをコピーして継承情報として登録
                    let binding = NgIncludeBinding {
                        parent_uri: uri.clone(),
                        template_path,
                        resolved_filename,
                        line: scope_start_line,
                        inherited_controllers: controller_stack.clone(),
                        inherited_local_variables,
                        inherited_form_bindings,
                    };
                    self.index.add_ng_include_binding(binding);
                }
            }

            // 子要素を再帰的に処理
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.collect_controller_scopes_and_includes_with_locals(
                    child,
                    source,
                    uri,
                    controller_stack,
                    local_var_stack,
                    form_binding_stack,
                );
            }

            // このノードで追加したコントローラーをスタックから削除
            if added_controller {
                controller_stack.pop();
            }

            // このノードで追加したローカル変数をスタックから削除
            for var_name in added_local_vars {
                if let Some(pos) = local_var_stack.iter().rposition(|v| v.name == var_name) {
                    local_var_stack.remove(pos);
                }
            }

            // このノードで追加したフォームバインディングをスタックから削除
            if let Some(form_name) = added_form_binding {
                if let Some(pos) = form_binding_stack.iter().rposition(|f| f.name == form_name) {
                    form_binding_stack.remove(pos);
                }
            }
        } else {
            // 子ノードを再帰的に処理
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.collect_controller_scopes_and_includes_with_locals(
                    child,
                    source,
                    uri,
                    controller_stack,
                    local_var_stack,
                    form_binding_stack,
                );
            }
        }
    }

    /// ng-repeatからローカル変数を抽出
    fn extract_local_vars_from_ng_repeat(
        &self,
        start_tag: Node,
        source: &str,
        scope_start_line: u32,
        scope_end_line: u32,
    ) -> Option<Vec<LocalVariableScope>> {
        let mut cursor = start_tag.walk();
        for child in start_tag.children(&mut cursor) {
            if child.kind() == "attribute" {
                if let Some(name_node) = self.find_child_by_kind(child, "attribute_name") {
                    let attr_name = self.node_text(name_node, source);

                    if attr_name == "ng-repeat" || attr_name == "data-ng-repeat" {
                        if let Some(value_node) =
                            self.find_child_by_kind(child, "quoted_attribute_value")
                        {
                            let raw_value = self.node_text(value_node, source);
                            let value = raw_value.trim_matches(|c| c == '"' || c == '\'');
                            let value_start_line = value_node.start_position().row as u32;
                            let value_start_col = value_node.start_position().column as u32 + 1;

                            return Some(self.parse_ng_repeat_vars(
                                value,
                                value_start_line,
                                value_start_col,
                                scope_start_line,
                                scope_end_line,
                            ));
                        }
                    }
                }
            }
        }
        None
    }

    /// ng-repeat式から変数をパース
    fn parse_ng_repeat_vars(
        &self,
        expr: &str,
        value_start_line: u32,
        value_start_col: u32,
        scope_start_line: u32,
        scope_end_line: u32,
    ) -> Vec<LocalVariableScope> {
        let mut result = Vec::new();

        let Some(in_idx) = expr.find(" in ") else {
            return result;
        };

        let iter_part = &expr[..in_idx];

        if iter_part.trim().starts_with('(') {
            // (key, value) 形式
            if let Some(open_paren) = iter_part.find('(') {
                if let Some(close_paren) = iter_part.find(')') {
                    let inner = &iter_part[open_paren + 1..close_paren];
                    let current_offset = open_paren + 1;

                    for var in inner.split(',') {
                        let var_trimmed = var.trim();
                        if !var_trimmed.is_empty() {
                            let var_offset_in_inner = var.as_ptr() as usize - inner.as_ptr() as usize;
                            let leading_spaces = var.len() - var.trim_start().len();
                            let offset = current_offset + var_offset_in_inner + leading_spaces;

                            result.push(LocalVariableScope {
                                name: var_trimmed.to_string(),
                                source: HtmlLocalVariableSource::NgRepeatKeyValue,
                                scope_start_line,
                                scope_end_line,
                                name_start_line: value_start_line,
                                name_start_col: value_start_col + offset as u32,
                                name_end_line: value_start_line,
                                name_end_col: value_start_col + offset as u32 + var_trimmed.len() as u32,
                            });
                        }
                    }
                }
            }
        } else {
            // item 形式
            let trimmed = iter_part.trim();
            if !trimmed.is_empty() {
                let leading_spaces = iter_part.len() - iter_part.trim_start().len();
                result.push(LocalVariableScope {
                    name: trimmed.to_string(),
                    source: HtmlLocalVariableSource::NgRepeatIterator,
                    scope_start_line,
                    scope_end_line,
                    name_start_line: value_start_line,
                    name_start_col: value_start_col + leading_spaces as u32,
                    name_end_line: value_start_line,
                    name_end_col: value_start_col + leading_spaces as u32 + trimmed.len() as u32,
                });
            }
        }

        result
    }

    /// ng-initからローカル変数を抽出
    fn extract_local_vars_from_ng_init(
        &self,
        start_tag: Node,
        source: &str,
        scope_start_line: u32,
        scope_end_line: u32,
    ) -> Option<Vec<LocalVariableScope>> {
        let mut cursor = start_tag.walk();
        for child in start_tag.children(&mut cursor) {
            if child.kind() == "attribute" {
                if let Some(name_node) = self.find_child_by_kind(child, "attribute_name") {
                    let attr_name = self.node_text(name_node, source);

                    if attr_name == "ng-init" || attr_name == "data-ng-init" {
                        if let Some(value_node) =
                            self.find_child_by_kind(child, "quoted_attribute_value")
                        {
                            let raw_value = self.node_text(value_node, source);
                            let value = raw_value.trim_matches(|c| c == '"' || c == '\'');
                            let value_start_line = value_node.start_position().row as u32;
                            let value_start_col = value_node.start_position().column as u32 + 1;

                            return Some(self.parse_ng_init_vars(
                                value,
                                value_start_line,
                                value_start_col,
                                scope_start_line,
                                scope_end_line,
                            ));
                        }
                    }
                }
            }
        }
        None
    }

    /// ng-init式から変数をパース
    fn parse_ng_init_vars(
        &self,
        expr: &str,
        value_start_line: u32,
        value_start_col: u32,
        scope_start_line: u32,
        scope_end_line: u32,
    ) -> Vec<LocalVariableScope> {
        let mut result = Vec::new();
        let mut pos = 0;

        for statement in expr.split(';') {
            if let Some(eq_idx) = statement.find('=') {
                let before_eq = &statement[..eq_idx];
                let after_eq_char = statement.chars().nth(eq_idx + 1);
                if after_eq_char != Some('=') && !before_eq.ends_with('!') {
                    let lhs = before_eq.trim();
                    if !lhs.is_empty() && self.is_valid_identifier(lhs) {
                        let leading_spaces = before_eq.len() - before_eq.trim_start().len();
                        let offset = pos + leading_spaces;

                        result.push(LocalVariableScope {
                            name: lhs.to_string(),
                            source: HtmlLocalVariableSource::NgInit,
                            scope_start_line,
                            scope_end_line,
                            name_start_line: value_start_line,
                            name_start_col: value_start_col + offset as u32,
                            name_end_line: value_start_line,
                            name_end_col: value_start_col + offset as u32 + lhs.len() as u32,
                        });
                    }
                }
            }
            pos += statement.len() + 1;
        }

        result
    }

    /// 有効な識別子かどうかをチェック
    pub(super) fn is_valid_identifier(&self, s: &str) -> bool {
        let mut chars = s.chars();
        if let Some(first) = chars.next() {
            if !first.is_alphabetic() && first != '_' && first != '$' {
                return false;
            }
            chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        } else {
            false
        }
    }

    /// <form name="x">からフォームバインディングを抽出
    /// 動的な name="{{...}}" はスキップする
    fn extract_form_name_from_tag(
        &self,
        start_tag: Node,
        source: &str,
        scope_start_line: u32,
        scope_end_line: u32,
    ) -> Option<FormBindingScope> {
        // タグ名を取得
        let tag_name_node = self.find_child_by_kind(start_tag, "tag_name")?;
        let tag_name = self.node_text(tag_name_node, source);

        // <form>タグのみ対象
        if tag_name != "form" {
            return None;
        }

        // name属性を探す
        let mut cursor = start_tag.walk();
        for child in start_tag.children(&mut cursor) {
            if child.kind() == "attribute" {
                if let Some(name_node) = self.find_child_by_kind(child, "attribute_name") {
                    let attr_name = self.node_text(name_node, source);

                    if attr_name == "name" {
                        if let Some(value_node) =
                            self.find_child_by_kind(child, "quoted_attribute_value")
                        {
                            let raw_value = self.node_text(value_node, source);
                            // クォートを除去
                            let value = raw_value.trim_matches(|c| c == '"' || c == '\'');

                            // 動的バインディング {{...}} をスキップ
                            if value.contains("{{") || value.contains("}}") {
                                return None;
                            }

                            // 空でない有効な識別子のみ
                            if value.is_empty() || !self.is_valid_identifier(value) {
                                return None;
                            }

                            // 値の位置を計算（クォートの次から）
                            let value_start_line = value_node.start_position().row as u32;
                            let value_start_col = value_node.start_position().column as u32 + 1; // クォート分

                            return Some(FormBindingScope {
                                name: value.to_string(),
                                scope_start_line,
                                scope_end_line,
                                name_start_line: value_start_line,
                                name_start_col: value_start_col,
                                name_end_line: value_start_line,
                                name_end_col: value_start_col + value.len() as u32,
                            });
                        }
                    }
                }
            }
        }
        None
    }

    /// ng-controller属性の値を取得
    /// 戻り値: (コントローラー名, alias)
    /// 例: "UserController as vm" -> ("UserController", Some("vm"))
    pub(super) fn get_ng_controller_attribute(&self, start_tag: Node, source: &str) -> Option<(String, Option<String>)> {
        let mut cursor = start_tag.walk();
        for child in start_tag.children(&mut cursor) {
            if child.kind() == "attribute" {
                if let Some(name) = self.find_child_by_kind(child, "attribute_name") {
                    let attr_name = self.node_text(name, source);
                    if attr_name == "ng-controller" || attr_name == "data-ng-controller" {
                        if let Some(value) = self.find_child_by_kind(child, "quoted_attribute_value") {
                            let raw_value = self.node_text(value, source);
                            // クォートを除去
                            let value = raw_value.trim_matches(|c| c == '"' || c == '\'');
                            // "Controller as alias"形式をパース
                            let parts: Vec<&str> = value.split_whitespace().collect();
                            let controller_name = parts.first().unwrap_or(&value).to_string();
                            let alias = if parts.len() >= 3 && parts[1].eq_ignore_ascii_case("as") {
                                Some(parts[2].to_string())
                            } else {
                                None
                            };
                            return Some((controller_name, alias));
                        }
                    }
                }
            }
        }
        None
    }

    /// ng-include属性またはsrc属性（<ng-include>要素用）の値を取得
    pub(super) fn get_ng_include_attribute(&self, start_tag: Node, source: &str) -> Option<String> {
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
    pub(super) fn extract_string_literal(&self, expr: &str) -> Option<String> {
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
}