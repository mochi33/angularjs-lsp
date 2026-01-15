//! ES6 export default 文の解析

use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::context::AnalyzerContext;
use super::AngularJsAnalyzer;
use crate::index::{ExportInfo, Symbol, SymbolKind};

impl AngularJsAnalyzer {
    /// ES6 export default 文を解析する
    ///
    /// 認識パターン:
    /// ```javascript
    /// export default ['UsersDataService', '$mdSidenav', AppController];
    /// export default AppController;
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
                    self.analyze_export_identifier(declaration, source, uri);
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

        // $scope がある場合、DI スコープを追加（$scope プロパティ追跡用）
        if has_scope {
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
    fn analyze_export_identifier(&self, node: Node, source: &str, uri: &Url) {
        let component_name = self.node_text(node, source);

        // ExportInfo を登録（依存関係なし）
        let export_info = ExportInfo {
            uri: uri.clone(),
            component_name: component_name.clone(),
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
            name: component_name.clone(),
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
}
