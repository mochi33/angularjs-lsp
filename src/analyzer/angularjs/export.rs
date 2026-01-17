//! ES6 export default 文の解析

use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::context::AnalyzerContext;
use super::AngularJsAnalyzer;
use crate::index::{ComponentTemplateUrl, ExportInfo, ExportedComponentObject, Symbol, SymbolKind};

impl AngularJsAnalyzer {
    /// ES6 export default 文を解析する
    ///
    /// 認識パターン:
    /// ```javascript
    /// export default ['UsersDataService', '$mdSidenav', AppController];
    /// export default AppController;
    /// export default { name: 'userDetails', config: {...} };  // AngularJS 1.5+ component pattern
    /// ```
    pub(super) fn analyze_export_statement(
        &self,
        node: Node,
        source: &str,
        uri: &Url,
        ctx: &mut AnalyzerContext,
    ) {
        // "export default" かどうかを確認
        let has_default = node.children(&mut node.walk()).any(|c| c.kind() == "default");

        if !has_default {
            return;
        }

        // エクスポートされる値を取得
        if let Some(declaration) = self.get_export_declaration(node) {
            match declaration.kind() {
                "array" => {
                    self.analyze_export_di_array(declaration, source, uri, ctx);
                }
                "identifier" => {
                    self.analyze_export_identifier(declaration, source, uri, ctx);
                }
                "object" => {
                    // export default { name: 'xxx', config: {...} } パターン
                    self.analyze_export_object(declaration, source, uri);
                }
                "function_expression" | "arrow_function" | "class" => {
                    // export default function() {} や export default class {}
                    // これらはDI情報なしでは追跡が困難
                }
                _ => {}
            }
        }
    }

    /// export文から宣言部分を取得
    fn get_export_declaration<'a>(&self, export_node: Node<'a>) -> Option<Node<'a>> {
        export_node.children(&mut export_node.walk()).find(|c| {
            matches!(
                c.kind(),
                "array"
                    | "identifier"
                    | "function_expression"
                    | "arrow_function"
                    | "class"
                    | "call_expression"
                    | "object"
            )
        })
    }

    /// export default ['dep1', 'dep2', FunctionRef] パターンを解析
    fn analyze_export_di_array(
        &self,
        array_node: Node,
        source: &str,
        uri: &Url,
        ctx: &mut AnalyzerContext,
    ) {
        if !self.is_di_array_pattern(array_node, source) {
            return;
        }

        // named_children で要素を取得（カンマをスキップ）
        let children: Vec<_> = array_node.named_children(&mut array_node.walk()).collect();

        if children.is_empty() {
            return;
        }

        let last = children.last().unwrap();

        // 依存関係を抽出（最後の要素以外の文字列）
        let dependencies: Vec<String> = children[..children.len() - 1]
            .iter()
            .filter(|c| c.kind() == "string")
            .map(|c| self.extract_string_value(*c, source))
            .collect();

        // コンポーネント名を取得
        let component_name = self.extract_component_name_from_export(*last, source, uri);

        // $scope と $rootScope のチェック
        let has_scope = dependencies.iter().any(|d| d == "$scope");
        let has_root_scope = dependencies.iter().any(|d| d == "$rootScope");

        // Angular以外の依存（サービス）を抽出
        let injected_services: Vec<String> = dependencies
            .iter()
            .filter(|d| !d.starts_with('$'))
            .cloned()
            .collect();

        // 依存関係として注入されるサービスへの参照を登録
        // 各DI文字列エントリに対応する参照を登録
        for (idx, child) in children[..children.len() - 1].iter().enumerate() {
            if child.kind() == "string" {
                let service_name = &dependencies[idx];
                // $で始まらないサービスを参照として登録
                if !service_name.starts_with('$') {
                    let start = child.start_position();
                    let end = child.end_position();
                    let reference = crate::index::SymbolReference {
                        name: service_name.clone(),
                        uri: uri.clone(),
                        start_line: self.offset_line(start.row as u32),
                        // 文字列リテラルのクォートの内側を参照位置とする
                        start_col: start.column as u32 + 1,
                        end_line: self.offset_line(end.row as u32),
                        end_col: if end.column > 0 { end.column as u32 - 1 } else { end.column as u32 },
                    };
                    self.index.add_reference(reference);
                }
            }
        }

        // ExportInfo を登録
        let export_info = ExportInfo {
            uri: uri.clone(),
            component_name: component_name.clone(),
            dependencies: dependencies.clone(),
            start_line: self.offset_line(array_node.start_position().row as u32),
            start_col: array_node.start_position().column as u32,
            end_line: self.offset_line(array_node.end_position().row as u32),
            end_col: array_node.end_position().column as u32,
            has_scope,
            has_root_scope,
        };
        self.index.add_export(export_info);

        // Symbol として登録（Go to Definition 用）
        let symbol = Symbol {
            name: component_name.clone(),
            kind: SymbolKind::ExportedComponent,
            uri: uri.clone(),
            start_line: self.offset_line(array_node.start_position().row as u32),
            start_col: array_node.start_position().column as u32,
            end_line: self.offset_line(array_node.end_position().row as u32),
            end_col: array_node.end_position().column as u32,
            name_start_line: self.offset_line(last.start_position().row as u32),
            name_start_col: last.start_position().column as u32,
            name_end_line: self.offset_line(last.end_position().row as u32),
            name_end_col: last.end_position().column as u32,
            docs: None,
            parameters: None,
        };
        self.index.add_definition(symbol);

        // メソッド/プロパティを抽出
        // Controller: this.method, self.method（エイリアス）
        // Service: this.method
        // Factory: return { method: ... }
        self.extract_exported_component_methods(*last, source, uri, &component_name);

        // DI スコープを追加（$scope プロパティ追跡用、またはサービス参照追跡用）
        // $scope がある場合、または injected_services が空でない場合にスコープをプッシュ
        if has_scope || has_root_scope || !injected_services.is_empty() {
            if let Some((body_start, body_end)) = self.find_function_body_range(*last, source) {
                ctx.push_scope(super::context::DiScope {
                    component_name: component_name.clone(),
                    injected_services,
                    body_start_line: body_start,
                    body_end_line: body_end,
                    has_scope,
                    has_root_scope,
                });
            }
        }
    }

    /// 配列がDIパターンかチェック: ['string', ..., function/identifier]
    fn is_di_array_pattern(&self, array_node: Node, _source: &str) -> bool {
        let children: Vec<_> = array_node.named_children(&mut array_node.walk()).collect();

        if children.is_empty() {
            return false;
        }

        let last = children.last().unwrap();
        let is_function_like = matches!(
            last.kind(),
            "function_expression" | "arrow_function" | "identifier" | "class"
        );

        if !is_function_like {
            return false;
        }

        // 最後の要素以外が全て文字列であること
        children[..children.len() - 1]
            .iter()
            .all(|c| c.kind() == "string")
    }

    /// export 関数参照からコンポーネント名を抽出
    fn extract_component_name_from_export(&self, node: Node, source: &str, uri: &Url) -> String {
        match node.kind() {
            "identifier" => {
                // 識別子名をそのまま使用
                self.node_text(node, source)
            }
            _ => {
                // ファイル名から拡張子を除いた名前をフォールバックとして使用
                let path = uri.path();
                let filename = path.rsplit('/').next().unwrap_or("anonymous");
                filename.trim_end_matches(".js").to_string()
            }
        }
    }

    /// export default からメソッド/プロパティを抽出
    ///
    /// 以下のパターンを全てサポート:
    /// - Controller: `this.method = ...`, `var self = this; self.method = ...`
    /// - Service: `this.method = ...`
    /// - Factory: `return { method: function() {} }`
    /// - ES6 Class: `class { method() {} }`
    fn extract_exported_component_methods(
        &self,
        node: Node,
        source: &str,
        uri: &Url,
        component_name: &str,
    ) {
        match node.kind() {
            "function_expression" | "arrow_function" => {
                // 関数式: export default ['deps', function() { ... }]
                self.extract_all_patterns_from_function(node, source, uri, component_name);
            }
            "class" => {
                // ES6 class式: export default ['deps', class { ... }]
                self.extract_methods_from_class(node, source, uri, component_name);
            }
            "identifier" => {
                // 関数参照: export default ['deps', FunctionRef]
                let ref_name = self.node_text(node, source);
                let root = {
                    let mut current = node;
                    while let Some(parent) = current.parent() {
                        current = parent;
                    }
                    current
                };

                // 関数宣言を探す
                if let Some(func_decl) = self.find_function_declaration(root, source, &ref_name) {
                    if let Some(body) = func_decl.child_by_field_name("body") {
                        self.extract_all_patterns_from_body(body, source, uri, component_name);
                    }
                } else if let Some(class_decl) = self.find_class_declaration(root, source, &ref_name)
                {
                    // class宣言
                    self.extract_methods_from_class(class_decl, source, uri, component_name);
                }
            }
            _ => {}
        }
    }

    /// 関数本体から全パターンのメソッド/プロパティを抽出
    fn extract_all_patterns_from_function(
        &self,
        func_node: Node,
        source: &str,
        uri: &Url,
        component_name: &str,
    ) {
        if let Some(body) = func_node.child_by_field_name("body") {
            self.extract_all_patterns_from_body(body, source, uri, component_name);
        }
    }

    /// 関数本体から全パターンを抽出
    ///
    /// 1. thisエイリアス（var self = this）を収集
    /// 2. this.method と self.method（エイリアス）パターンを抽出
    /// 3. return { method: ... } パターンを抽出
    fn extract_all_patterns_from_body(
        &self,
        body: Node,
        source: &str,
        uri: &Url,
        component_name: &str,
    ) {
        // thisエイリアスを収集
        let this_aliases = self.collect_this_aliases(body, source);

        // ローカル変数/関数宣言の位置を収集（factory return用）
        let mut local_vars = std::collections::HashMap::new();
        self.collect_local_vars_for_export(body, source, &mut local_vars);

        // 全パターンをスキャン
        self.scan_all_patterns(body, source, uri, component_name, &this_aliases, &local_vars);
    }

    /// ローカル変数/関数宣言を収集
    fn collect_local_vars_for_export(
        &self,
        node: Node,
        source: &str,
        local_vars: &mut std::collections::HashMap<String, (u32, u32, u32, u32)>,
    ) {
        match node.kind() {
            "function_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let func_name = self.node_text(name_node, source);
                    let start = node.start_position();
                    let end = node.end_position();
                    local_vars.insert(
                        func_name,
                        (
                            self.offset_line(start.row as u32),
                            start.column as u32,
                            self.offset_line(end.row as u32),
                            end.column as u32,
                        ),
                    );
                }
            }
            "variable_declaration" | "lexical_declaration" => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "variable_declarator" {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            if let Some(value_node) = child.child_by_field_name("value") {
                                if matches!(
                                    value_node.kind(),
                                    "function_expression" | "arrow_function"
                                ) {
                                    let var_name = self.node_text(name_node, source);
                                    let start = value_node.start_position();
                                    let end = value_node.end_position();
                                    local_vars.insert(
                                        var_name,
                                        (
                                            self.offset_line(start.row as u32),
                                            start.column as u32,
                                            self.offset_line(end.row as u32),
                                            end.column as u32,
                                        ),
                                    );
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.collect_local_vars_for_export(child, source, local_vars);
        }
    }

    /// 全パターンをスキャン
    fn scan_all_patterns(
        &self,
        node: Node,
        source: &str,
        uri: &Url,
        component_name: &str,
        this_aliases: &[String],
        local_vars: &std::collections::HashMap<String, (u32, u32, u32, u32)>,
    ) {
        match node.kind() {
            "expression_statement" => {
                if let Some(expr) = node.named_child(0) {
                    if expr.kind() == "assignment_expression" {
                        // this.method と self.method（エイリアス）パターン
                        self.extract_this_or_alias_method_for_export(
                            expr,
                            source,
                            uri,
                            component_name,
                            this_aliases,
                        );
                    }
                }
            }
            "return_statement" => {
                // return { method: ... } パターン（Factory）
                if let Some(arg) = node.named_child(0) {
                    if arg.kind() == "object" {
                        self.extract_factory_methods_for_export(
                            arg,
                            source,
                            uri,
                            component_name,
                            local_vars,
                        );
                    }
                }
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.scan_all_patterns(child, source, uri, component_name, this_aliases, local_vars);
        }
    }

    /// this.method または self.method（エイリアス）パターンを抽出
    fn extract_this_or_alias_method_for_export(
        &self,
        assign_node: Node,
        source: &str,
        uri: &Url,
        component_name: &str,
        this_aliases: &[String],
    ) {
        if let Some(left) = assign_node.child_by_field_name("left") {
            if left.kind() == "member_expression" {
                if let Some(object) = left.child_by_field_name("object") {
                    let obj_text = self.node_text(object, source);

                    // this または thisエイリアス（self, vm等）かどうか
                    let is_this_or_alias = obj_text == "this" || this_aliases.contains(&obj_text);

                    if is_this_or_alias {
                        if let Some(property) = left.child_by_field_name("property") {
                            let method_name = self.node_text(property, source);
                            let start = property.start_position();
                            let end = property.end_position();

                            let docs =
                                self.extract_jsdoc_for_line(assign_node.start_position().row, source);

                            let parameters = assign_node
                                .child_by_field_name("right")
                                .and_then(|right| self.extract_function_params(right, source));

                            let full_name = format!("{}.{}", component_name, method_name);
                            let symbol = Symbol {
                                name: full_name,
                                kind: SymbolKind::Method,
                                uri: uri.clone(),
                                start_line: self.offset_line(start.row as u32),
                                start_col: start.column as u32,
                                end_line: self.offset_line(end.row as u32),
                                end_col: end.column as u32,
                                name_start_line: self.offset_line(start.row as u32),
                                name_start_col: start.column as u32,
                                name_end_line: self.offset_line(end.row as u32),
                                name_end_col: end.column as u32,
                                docs,
                                parameters,
                            };
                            self.index.add_definition(symbol);
                        }
                    }
                }
            }
        }
    }

    /// return { method: ... } パターン（Factory）を抽出
    fn extract_factory_methods_for_export(
        &self,
        obj_node: Node,
        source: &str,
        uri: &Url,
        component_name: &str,
        local_vars: &std::collections::HashMap<String, (u32, u32, u32, u32)>,
    ) {
        let mut cursor = obj_node.walk();
        for child in obj_node.children(&mut cursor) {
            match child.kind() {
                "pair" => {
                    if let Some(key) = child.child_by_field_name("key") {
                        if let Some(value) = child.child_by_field_name("value") {
                            let method_name = self.node_text(key, source);
                            let full_name = format!("{}.{}", component_name, method_name);
                            let name_start = key.start_position();
                            let name_end = key.end_position();

                            match value.kind() {
                                "function_expression" | "arrow_function" => {
                                    let start = key.start_position();
                                    let end = key.end_position();
                                    let docs = self
                                        .extract_jsdoc_for_line(child.start_position().row, source);
                                    let parameters = self.extract_function_params(value, source);
                                    let symbol = Symbol {
                                        name: full_name,
                                        kind: SymbolKind::Method,
                                        uri: uri.clone(),
                                        start_line: self.offset_line(start.row as u32),
                                        start_col: start.column as u32,
                                        end_line: self.offset_line(end.row as u32),
                                        end_col: end.column as u32,
                                        name_start_line: self.offset_line(name_start.row as u32),
                                        name_start_col: name_start.column as u32,
                                        name_end_line: self.offset_line(name_end.row as u32),
                                        name_end_col: name_end.column as u32,
                                        docs,
                                        parameters,
                                    };
                                    self.index.add_definition(symbol);
                                }
                                "identifier" => {
                                    let var_name = self.node_text(value, source);
                                    if let Some((start_line, start_col, end_line, end_col)) =
                                        local_vars.get(&var_name)
                                    {
                                        let docs =
                                            self.extract_jsdoc_for_line(*start_line as usize, source);
                                        let symbol = Symbol {
                                            name: full_name,
                                            kind: SymbolKind::Method,
                                            uri: uri.clone(),
                                            start_line: *start_line,
                                            start_col: *start_col,
                                            end_line: *end_line,
                                            end_col: *end_col,
                                            name_start_line: self
                                                .offset_line(name_start.row as u32),
                                            name_start_col: name_start.column as u32,
                                            name_end_line: self.offset_line(name_end.row as u32),
                                            name_end_col: name_end.column as u32,
                                            docs,
                                            parameters: None,
                                        };
                                        self.index.add_definition(symbol);
                                    } else {
                                        let start = key.start_position();
                                        let end = key.end_position();
                                        let docs = self
                                            .extract_jsdoc_for_line(child.start_position().row, source);
                                        let symbol = Symbol {
                                            name: full_name,
                                            kind: SymbolKind::Method,
                                            uri: uri.clone(),
                                            start_line: self.offset_line(start.row as u32),
                                            start_col: start.column as u32,
                                            end_line: self.offset_line(end.row as u32),
                                            end_col: end.column as u32,
                                            name_start_line: self
                                                .offset_line(name_start.row as u32),
                                            name_start_col: name_start.column as u32,
                                            name_end_line: self.offset_line(name_end.row as u32),
                                            name_end_col: name_end.column as u32,
                                            docs,
                                            parameters: None,
                                        };
                                        self.index.add_definition(symbol);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                "shorthand_property_identifier" => {
                    let method_name = self.node_text(child, source);
                    let full_name = format!("{}.{}", component_name, method_name);
                    let name_start = child.start_position();
                    let name_end = child.end_position();

                    if let Some((start_line, start_col, end_line, end_col)) =
                        local_vars.get(&method_name)
                    {
                        let docs = self.extract_jsdoc_for_line(*start_line as usize, source);
                        let symbol = Symbol {
                            name: full_name,
                            kind: SymbolKind::Method,
                            uri: uri.clone(),
                            start_line: *start_line,
                            start_col: *start_col,
                            end_line: *end_line,
                            end_col: *end_col,
                            name_start_line: self.offset_line(name_start.row as u32),
                            name_start_col: name_start.column as u32,
                            name_end_line: self.offset_line(name_end.row as u32),
                            name_end_col: name_end.column as u32,
                            docs,
                            parameters: None,
                        };
                        self.index.add_definition(symbol);
                    } else {
                        let start = child.start_position();
                        let end = child.end_position();
                        let docs = self.extract_jsdoc_for_line(start.row, source);
                        let symbol = Symbol {
                            name: full_name,
                            kind: SymbolKind::Method,
                            uri: uri.clone(),
                            start_line: self.offset_line(start.row as u32),
                            start_col: start.column as u32,
                            end_line: self.offset_line(end.row as u32),
                            end_col: end.column as u32,
                            name_start_line: self.offset_line(name_start.row as u32),
                            name_start_col: name_start.column as u32,
                            name_end_line: self.offset_line(name_end.row as u32),
                            name_end_col: name_end.column as u32,
                            docs,
                            parameters: None,
                        };
                        self.index.add_definition(symbol);
                    }
                }
                _ => {}
            }
        }
    }

    /// export default Identifier パターンを解析
    ///
    /// 識別子が参照している実体を見て処理を分岐する:
    /// - 配列: DI配列パターンとして処理
    /// - オブジェクト: コンポーネントオブジェクトとして処理
    /// - クラス: クラスメソッドを抽出
    /// - 関数: 関数メソッドを抽出
    fn analyze_export_identifier(&self, node: Node, source: &str, uri: &Url, ctx: &mut AnalyzerContext) {
        let identifier_name = self.node_text(node, source);

        // ルートノードを取得
        let root = {
            let mut current = node;
            while let Some(parent) = current.parent() {
                current = parent;
            }
            current
        };

        // 変数宣言を探す
        if let Some(value_node) = self.find_variable_value(root, source, &identifier_name) {
            match value_node.kind() {
                "array" => {
                    // 配列の場合、DI配列パターンとして処理
                    self.analyze_export_di_array(value_node, source, uri, ctx);
                    return;
                }
                "object" => {
                    // オブジェクトの場合、コンポーネントオブジェクトとして処理
                    self.analyze_export_object(value_node, source, uri);
                    return;
                }
                _ => {}
            }
        }

        // クラス宣言を探してメソッドを抽出
        if let Some(class_decl) = self.find_class_declaration(root, source, &identifier_name) {
            self.extract_methods_from_class(class_decl, source, uri, &identifier_name);
        }

        // ExportInfo を登録（依存関係なし）
        let export_info = ExportInfo {
            uri: uri.clone(),
            component_name: identifier_name.clone(),
            dependencies: Vec::new(),
            start_line: self.offset_line(node.start_position().row as u32),
            start_col: node.start_position().column as u32,
            end_line: self.offset_line(node.end_position().row as u32),
            end_col: node.end_position().column as u32,
            has_scope: false,
            has_root_scope: false,
        };
        self.index.add_export(export_info);

        // Symbol として登録
        let symbol = Symbol {
            name: identifier_name.clone(),
            kind: SymbolKind::ExportedComponent,
            uri: uri.clone(),
            start_line: self.offset_line(node.start_position().row as u32),
            start_col: node.start_position().column as u32,
            end_line: self.offset_line(node.end_position().row as u32),
            end_col: node.end_position().column as u32,
            name_start_line: self.offset_line(node.start_position().row as u32),
            name_start_col: node.start_position().column as u32,
            name_end_line: self.offset_line(node.end_position().row as u32),
            name_end_col: node.end_position().column as u32,
            docs: None,
            parameters: None,
        };
        self.index.add_definition(symbol);
    }

    /// export default { name: 'xxx', config: {...} } パターンを解析
    ///
    /// AngularJS 1.5+ のコンポーネントモジュールパターン用
    /// ```javascript
    /// // UserDetails.js
    /// export default {
    ///   name : 'userDetails',
    ///   config : {
    ///     bindings: { selected: '<' },
    ///     templateUrl: 'src/users/components/details/UserDetails.html',
    ///     controller: [ '$mdBottomSheet', '$log', UserDetailsController ]
    ///   }
    /// };
    /// ```
    fn analyze_export_object(&self, obj_node: Node, source: &str, uri: &Url) {
        let mut component_name: Option<String> = None;
        let mut name_node: Option<Node> = None;
        let mut config_node: Option<Node> = None;

        // オブジェクトのプロパティを走査
        let mut cursor = obj_node.walk();
        for child in obj_node.children(&mut cursor) {
            if child.kind() == "pair" {
                if let Some(key) = child.child_by_field_name("key") {
                    let key_text = self.node_text(key, source);
                    // 識別子の場合はそのまま、文字列の場合はクォートを除去
                    let key_name = key_text.trim_matches(|c| c == '"' || c == '\'');

                    if key_name == "name" {
                        if let Some(value) = child.child_by_field_name("value") {
                            if value.kind() == "string" {
                                component_name = Some(self.extract_string_value(value, source));
                                name_node = Some(value);
                            }
                        }
                    } else if key_name == "config" {
                        if let Some(value) = child.child_by_field_name("value") {
                            if value.kind() == "object" {
                                config_node = Some(value);
                            }
                        }
                    }
                }
            }
        }

        // config オブジェクトから templateUrl, bindings を抽出
        if let Some(config) = config_node {
            self.extract_template_url_from_config(config, source, uri, component_name.as_deref());
        }

        // name プロパティが見つかった場合、ExportedComponentObject として登録
        if let (Some(name), Some(name_pos)) = (component_name, name_node) {
            let exported_obj = ExportedComponentObject {
                uri: uri.clone(),
                name: name.clone(),
                start_line: self.offset_line(obj_node.start_position().row as u32),
                start_col: obj_node.start_position().column as u32,
                end_line: self.offset_line(obj_node.end_position().row as u32),
                end_col: obj_node.end_position().column as u32,
            };
            self.index.add_exported_component_object(exported_obj);

            // Symbol としても登録（Go to Definition 用）
            let symbol = Symbol {
                name,
                kind: SymbolKind::Component,
                uri: uri.clone(),
                start_line: self.offset_line(obj_node.start_position().row as u32),
                start_col: obj_node.start_position().column as u32,
                end_line: self.offset_line(obj_node.end_position().row as u32),
                end_col: obj_node.end_position().column as u32,
                name_start_line: self.offset_line(name_pos.start_position().row as u32),
                name_start_col: name_pos.start_position().column as u32,
                name_end_line: self.offset_line(name_pos.end_position().row as u32),
                name_end_col: name_pos.end_position().column as u32,
                docs: None,
                parameters: None,
            };
            self.index.add_definition(symbol);
        }
    }

    /// config オブジェクトから templateUrl, controller, controllerAs, bindings を抽出
    fn extract_template_url_from_config(&self, config_node: Node, source: &str, uri: &Url, component_name: Option<&str>) {
        let mut template_path: Option<String> = None;
        let mut template_line: Option<u32> = None;
        let mut template_col: Option<u32> = None;
        let mut controller_name: Option<String> = None;
        let mut controller_as: Option<String> = None;
        let mut bindings_node: Option<Node> = None;

        let mut cursor = config_node.walk();
        for child in config_node.children(&mut cursor) {
            if child.kind() == "pair" {
                if let Some(key) = child.child_by_field_name("key") {
                    let key_text = self.node_text(key, source);
                    let key_name = key_text.trim_matches(|c| c == '"' || c == '\'');

                    if let Some(value) = child.child_by_field_name("value") {
                        match key_name {
                            "templateUrl" => {
                                if value.kind() == "string" {
                                    template_path = Some(self.extract_string_value(value, source));
                                    let start = value.start_position();
                                    template_line = Some(self.offset_line(start.row as u32));
                                    template_col = Some(start.column as u32);
                                }
                            }
                            "controller" => {
                                // controller: 'ControllerName' (文字列参照)
                                if value.kind() == "string" {
                                    controller_name = Some(self.extract_string_value(value, source));
                                }
                                // controller: ControllerName (識別子参照)
                                else if value.kind() == "identifier" {
                                    controller_name = Some(self.node_text(value, source).to_string());
                                }
                                // controller: ['$dep1', '$dep2', ControllerName] (DI配列パターン)
                                else if value.kind() == "array" {
                                    // 配列の最後の要素がコントローラー
                                    let mut cursor = value.walk();
                                    let mut last_element: Option<tree_sitter::Node> = None;
                                    for child in value.children(&mut cursor) {
                                        if child.is_named() {
                                            last_element = Some(child);
                                        }
                                    }
                                    if let Some(last) = last_element {
                                        if last.kind() == "identifier" {
                                            controller_name = Some(self.node_text(last, source).to_string());
                                        }
                                    }
                                }
                            }
                            "controllerAs" => {
                                if value.kind() == "string" {
                                    controller_as = Some(self.extract_string_value(value, source));
                                }
                            }
                            "bindings" => {
                                if value.kind() == "object" {
                                    bindings_node = Some(value);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // コントローラー名がない場合はコンポーネント名を使用
        // これにより $ctrl.xxx でバインディングにアクセス可能になる
        let effective_controller_name = controller_name.clone().or_else(|| component_name.map(|s| s.to_string()));

        // templateUrlが存在する場合のみ登録
        if let (Some(path), Some(line), Some(col)) = (template_path, template_line, template_col) {
            let template_url = ComponentTemplateUrl {
                uri: uri.clone(),
                template_path: path,
                line,
                col,
                controller_name: effective_controller_name.clone(),
                // controllerAs が指定されていない場合は "$ctrl" がデフォルト
                controller_as: controller_as.unwrap_or_else(|| "$ctrl".to_string()),
            };
            self.index.add_component_template_url(template_url);
        }

        // bindings を抽出してシンボルとして登録
        if let (Some(bindings), Some(prefix)) = (bindings_node, effective_controller_name.as_deref()) {
            self.extract_bindings_from_config(bindings, source, uri, prefix);
        }
    }

    /// bindingsオブジェクトからバインディングを抽出してシンボルとして登録
    fn extract_bindings_from_config(&self, bindings_node: Node, source: &str, uri: &Url, controller_name: &str) {
        let mut cursor = bindings_node.walk();
        for child in bindings_node.children(&mut cursor) {
            if child.kind() == "pair" {
                if let Some(key) = child.child_by_field_name("key") {
                    let key_text = self.node_text(key, source);
                    let binding_name = key_text.trim_matches(|c| c == '"' || c == '\'');

                    // バインディングタイプを取得
                    let binding_type = if let Some(value) = child.child_by_field_name("value") {
                        if value.kind() == "string" {
                            Some(self.extract_string_value(value, source))
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    let start = key.start_position();
                    let end = key.end_position();

                    let full_name = format!("{}.{}", controller_name, binding_name);
                    let docs = binding_type.map(|t| format!("Component binding: {}", t));

                    let symbol = Symbol {
                        name: full_name,
                        kind: SymbolKind::ComponentBinding,
                        uri: uri.clone(),
                        start_line: self.offset_line(start.row as u32),
                        start_col: start.column as u32,
                        end_line: self.offset_line(end.row as u32),
                        end_col: end.column as u32,
                        name_start_line: self.offset_line(start.row as u32),
                        name_start_col: start.column as u32,
                        name_end_line: self.offset_line(end.row as u32),
                        name_end_col: end.column as u32,
                        docs,
                        parameters: None,
                    };

                    self.index.add_definition(symbol);
                }
            }
        }
    }

    /// 指定された名前の変数宣言を探し、その値ノードを返す
    ///
    /// 対応するパターン:
    /// - `const config = [...];`
    /// - `let config = {...};`
    /// - `var config = [...];`
    fn find_variable_value<'a>(&self, root: Node<'a>, source: &str, var_name: &str) -> Option<Node<'a>> {
        self.find_variable_value_recursive(root, source, var_name)
    }

    fn find_variable_value_recursive<'a>(&self, node: Node<'a>, source: &str, var_name: &str) -> Option<Node<'a>> {
        match node.kind() {
            "variable_declaration" | "lexical_declaration" => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "variable_declarator" {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            if self.node_text(name_node, source) == var_name {
                                return child.child_by_field_name("value");
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = self.find_variable_value_recursive(child, source, var_name) {
                return Some(found);
            }
        }

        None
    }
}
