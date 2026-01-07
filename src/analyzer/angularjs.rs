use std::collections::HashMap;
use std::sync::Arc;

use tower_lsp::lsp_types::Url;
use tree_sitter::{Node, Tree};

use super::JsParser;
use crate::index::{Symbol, SymbolIndex, SymbolKind, SymbolReference};

/// ローカル変数/関数の定義位置
#[derive(Clone)]
struct LocalVarLocation {
    start_line: u32,
    start_col: u32,
    end_line: u32,
    end_col: u32,
}

/// コンポーネントのDIスコープ情報
#[derive(Clone, Debug)]
struct DiScope {
    /// コンポーネント名
    #[allow(dead_code)]
    component_name: String,
    /// DIされた依存サービス名のリスト
    injected_services: Vec<String>,
    /// 関数本体の開始行
    body_start_line: u32,
    /// 関数本体の終了行
    body_end_line: u32,
}

/// 解析コンテキスト
struct AnalyzerContext {
    /// 現在有効なDIスコープのスタック
    di_scopes: Vec<DiScope>,
    /// $inject パターン用: 関数名 -> DI依存関係
    inject_map: HashMap<String, Vec<String>>,
    /// $inject パターン用: 関数名 -> 関数本体の範囲 (start_line, end_line)
    function_ranges: HashMap<String, (u32, u32)>,
}

impl AnalyzerContext {
    fn new() -> Self {
        Self {
            di_scopes: Vec::new(),
            inject_map: HashMap::new(),
            function_ranges: HashMap::new(),
        }
    }

    /// 指定位置でサービスがDIされているかどうかをチェック
    fn is_injected_at(&self, service_name: &str, line: u32) -> bool {
        // 1. di_scopes から現在位置のスコープを探す（内側から外側へ）
        for scope in self.di_scopes.iter().rev() {
            if line >= scope.body_start_line && line <= scope.body_end_line {
                return scope.injected_services.iter().any(|s| s == service_name);
            }
        }

        // 2. $inject パターンのスコープもチェック
        for (func_name, range) in &self.function_ranges {
            if line >= range.0 && line <= range.1 {
                if let Some(deps) = self.inject_map.get(func_name) {
                    return deps.iter().any(|s| s == service_name);
                }
            }
        }

        false
    }

    /// DIスコープを追加
    fn push_scope(&mut self, scope: DiScope) {
        self.di_scopes.push(scope);
    }

    /// DIスコープを削除
    fn pop_scope(&mut self) {
        self.di_scopes.pop();
    }
}

/// AngularJS 1.x のコードを解析し、シンボル定義と参照を抽出するアナライザー
pub struct AngularJsAnalyzer {
    index: Arc<SymbolIndex>,
}

impl AngularJsAnalyzer {
    pub fn new(index: Arc<SymbolIndex>) -> Self {
        Self { index }
    }

    /// ドキュメントを解析してシンボルをインデックスに追加する
    ///
    /// 既存のドキュメント情報をクリアしてから解析を行う
    pub fn analyze_document(&self, uri: &Url, source: &str) {
        self.analyze_document_with_options(uri, source, true);
    }

    /// ドキュメントを解析してシンボルをインデックスに追加する
    ///
    /// # Arguments
    /// * `uri` - ドキュメントのURI
    /// * `source` - ソースコード
    /// * `clear` - true: 既存情報をクリア, false: 追記モード（2パス目用）
    pub fn analyze_document_with_options(&self, uri: &Url, source: &str, clear: bool) {
        let mut parser = JsParser::new();

        if let Some(tree) = parser.parse(source) {
            if clear {
                self.index.clear_document(uri);
            }
            let mut ctx = AnalyzerContext::new();
            // 事前収集: $inject パターン用の関数宣言と $inject パターンを収集
            self.collect_function_declarations_for_inject(tree.root_node(), source, &mut ctx);
            self.collect_inject_patterns(tree.root_node(), source, &mut ctx);
            self.traverse_tree(&tree, source, uri, &mut ctx);
        }
    }

    /// AST全体を走査する
    fn traverse_tree(&self, tree: &Tree, source: &str, uri: &Url, ctx: &mut AnalyzerContext) {
        let root = tree.root_node();
        self.visit_node(root, source, uri, ctx);
    }

    /// ASTノードを訪問し、種類に応じた解析を行う
    ///
    /// 認識するノード:
    /// - `call_expression`: 関数呼び出し（angular.module(), .controller() 等）
    /// - `member_expression`: プロパティアクセス（Service.method）
    /// - `expression_statement`: 式文（$inject パターン）
    /// - `identifier`: 識別子（サービス名等の参照）
    fn visit_node(&self, node: Node, source: &str, uri: &Url, ctx: &mut AnalyzerContext) {
        match node.kind() {
            "call_expression" => {
                self.analyze_call_expression(node, source, uri, ctx);
                self.analyze_method_call(node, source, uri, ctx);
            }
            "member_expression" => {
                self.analyze_member_access(node, source, uri, ctx);
            }
            "expression_statement" => {
                self.analyze_inject_pattern(node, source, uri);
            }
            "identifier" => {
                self.analyze_identifier(node, source, uri, ctx);
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit_node(child, source, uri, ctx);
        }
    }

    /// `$inject` パターンを解析する
    ///
    /// 認識パターン:
    /// ```javascript
    /// MyController.$inject = ['$scope', 'MyService'];
    /// ```
    fn analyze_inject_pattern(&self, node: Node, source: &str, uri: &Url) {
        if let Some(expr) = node.named_child(0) {
            if expr.kind() == "assignment_expression" {
                if let Some(left) = expr.child_by_field_name("left") {
                    if left.kind() == "member_expression" {
                        if let Some(property) = left.child_by_field_name("property") {
                            let prop_name = self.node_text(property, source);
                            if prop_name == "$inject" {
                                if let Some(right) = expr.child_by_field_name("right") {
                                    self.extract_inject_dependencies(right, source, uri);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// `$inject` 配列から依存サービスを抽出する
    ///
    /// `$` で始まるAngular組み込みサービスはスキップ
    fn extract_inject_dependencies(&self, node: Node, source: &str, uri: &Url) {
        if node.kind() == "array" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "string" {
                    let dep_name = self.extract_string_value(child, source);
                    if !dep_name.starts_with('$') {
                        let start = child.start_position();
                        let end = child.end_position();

                        let reference = SymbolReference {
                            name: dep_name,
                            uri: uri.clone(),
                            start_line: start.row as u32,
                            start_col: start.column as u32,
                            end_line: end.row as u32,
                            end_col: end.column as u32,
                        };

                        self.index.add_reference(reference);
                    }
                }
            }
        }
    }

    /// メソッド呼び出しを解析し、参照として登録する
    ///
    /// 認識パターン:
    /// ```javascript
    /// UserService.getAll()
    /// AuthService.login(credentials)
    /// ```
    ///
    /// `$xxx`, `this`, `console` はスキップ
    /// DIされていないサービスへのアクセスは参照として登録しない
    fn analyze_method_call(&self, node: Node, source: &str, uri: &Url, ctx: &AnalyzerContext) {
        if let Some(callee) = node.child_by_field_name("function") {
            if callee.kind() == "member_expression" {
                if let Some(object) = callee.child_by_field_name("object") {
                    if let Some(property) = callee.child_by_field_name("property") {
                        let obj_name = self.node_text(object, source);
                        let method_name = self.node_text(property, source);

                        if obj_name.starts_with('$') || obj_name == "this" || obj_name == "console" {
                            return;
                        }

                        // DIチェック: このスコープでサービスがDIされているか確認
                        let current_line = node.start_position().row as u32;
                        if !ctx.is_injected_at(&obj_name, current_line) {
                            return;
                        }

                        let full_name = format!("{}.{}", obj_name, method_name);

                        if self.index.has_definition(&full_name) {
                            let start = property.start_position();
                            let end = property.end_position();

                            let reference = SymbolReference {
                                name: full_name,
                                uri: uri.clone(),
                                start_line: start.row as u32,
                                start_col: start.column as u32,
                                end_line: end.row as u32,
                                end_col: end.column as u32,
                            };

                            self.index.add_reference(reference);
                        }
                    }
                }
            }
        }
    }

    /// メンバーアクセス（呼び出し以外）を解析し、参照として登録する
    ///
    /// 認識パターン:
    /// ```javascript
    /// var fn = UserService.getAll;  // 関数参照
    /// callback(AuthService.login);   // コールバック渡し
    /// ```
    ///
    /// DIされていないサービスへのアクセスは参照として登録しない
    fn analyze_member_access(&self, node: Node, source: &str, uri: &Url, ctx: &AnalyzerContext) {
        if let Some(object) = node.child_by_field_name("object") {
            if let Some(property) = node.child_by_field_name("property") {
                let obj_name = self.node_text(object, source);
                let prop_name = self.node_text(property, source);

                if obj_name.starts_with('$') || obj_name == "this" || obj_name == "console" {
                    return;
                }

                // DIチェック: このスコープでサービスがDIされているか確認
                let current_line = node.start_position().row as u32;
                if !ctx.is_injected_at(&obj_name, current_line) {
                    return;
                }

                let full_name = format!("{}.{}", obj_name, prop_name);

                if self.index.has_definition(&full_name) {
                    let start = property.start_position();
                    let end = property.end_position();

                    let reference = SymbolReference {
                        name: full_name,
                        uri: uri.clone(),
                        start_line: start.row as u32,
                        start_col: start.column as u32,
                        end_line: end.row as u32,
                        end_col: end.column as u32,
                    };

                    self.index.add_reference(reference);
                }
            }
        }
    }

    /// AngularJSのコンポーネント定義呼び出しを解析する
    ///
    /// 認識パターン:
    /// - `angular.module('name', [deps])` - モジュール定義
    /// - `.controller('Name', ...)` - コントローラー定義
    /// - `.service('Name', ...)` - サービス定義
    /// - `.factory('Name', ...)` - ファクトリー定義
    /// - `.directive('Name', ...)` - ディレクティブ定義
    /// - `.provider('Name', ...)` - プロバイダー定義
    /// - `.filter('Name', ...)` - フィルター定義
    /// - `.constant('Name', ...)` - 定数定義
    /// - `.value('Name', ...)` - 値定義
    fn analyze_call_expression(&self, node: Node, source: &str, uri: &Url, ctx: &mut AnalyzerContext) {
        if let Some(callee) = node.child_by_field_name("function") {
            let callee_text = self.node_text(callee, source);

            if callee_text == "angular.module" {
                self.extract_module_definition(node, source, uri);
            }

            if callee.kind() == "member_expression" {
                if let Some(property) = callee.child_by_field_name("property") {
                    let method_name = self.node_text(property, source);
                    match method_name.as_str() {
                        "controller" => self.extract_component_definition(node, source, uri, SymbolKind::Controller, ctx),
                        "service" => self.extract_component_definition(node, source, uri, SymbolKind::Service, ctx),
                        "factory" => self.extract_component_definition(node, source, uri, SymbolKind::Factory, ctx),
                        "directive" => self.extract_component_definition(node, source, uri, SymbolKind::Directive, ctx),
                        "provider" => self.extract_component_definition(node, source, uri, SymbolKind::Provider, ctx),
                        "filter" => self.extract_component_definition(node, source, uri, SymbolKind::Filter, ctx),
                        "constant" => self.extract_component_definition(node, source, uri, SymbolKind::Constant, ctx),
                        "value" => self.extract_component_definition(node, source, uri, SymbolKind::Value, ctx),
                        "config" | "run" => {}
                        _ => {}
                    }
                }
            }
        }
    }

    /// `angular.module()` からモジュール定義を抽出する
    ///
    /// 認識パターン:
    /// ```javascript
    /// angular.module('myApp', ['dep1', 'dep2'])
    /// angular.module('myApp')  // 既存モジュール参照
    /// ```
    fn extract_module_definition(&self, node: Node, source: &str, uri: &Url) {
        if let Some(args) = node.child_by_field_name("arguments") {
            if let Some(first_arg) = args.named_child(0) {
                if first_arg.kind() == "string" {
                    let name = self.extract_string_value(first_arg, source);
                    let start = first_arg.start_position();
                    let end = first_arg.end_position();

                    let docs = self.extract_jsdoc_for_line(start.row, source);
                    let symbol = Symbol {
                        name: name.clone(),
                        kind: SymbolKind::Module,
                        uri: uri.clone(),
                        // 定義位置とシンボル名位置は同じ（文字列リテラル）
                        start_line: start.row as u32,
                        start_col: start.column as u32,
                        end_line: end.row as u32,
                        end_col: end.column as u32,
                        name_start_line: start.row as u32,
                        name_start_col: start.column as u32,
                        name_end_line: end.row as u32,
                        name_end_col: end.column as u32,
                        docs,
                    };

                    self.index.add_definition(symbol);
                }
            }
        }
    }

    /// コンポーネント（controller, service, factory等）の定義を抽出する
    ///
    /// 認識パターン:
    /// ```javascript
    /// .controller('MyCtrl', ['$scope', 'Svc', function($scope, Svc) {}])
    /// .service('MySvc', function() {})
    /// ```
    ///
    /// service/factory の場合は内部メソッドも抽出する
    /// DIスコープを追加して、関数本体内でのDIチェックを可能にする
    fn extract_component_definition(&self, node: Node, source: &str, uri: &Url, kind: SymbolKind, ctx: &mut AnalyzerContext) {
        if let Some(args) = node.child_by_field_name("arguments") {
            if let Some(first_arg) = args.named_child(0) {
                if first_arg.kind() == "string" {
                    let component_name = self.extract_string_value(first_arg, source);

                    // シンボル名の位置（検索用）は常に文字列リテラルの位置
                    let name_start = first_arg.start_position();
                    let name_end = first_arg.end_position();

                    // 定義位置は関数定義を優先する
                    let (start, end, docs_line) = if let Some(second_arg) = args.named_child(1) {
                        self.extract_dependencies(second_arg, source, uri);

                        // DIスコープを追加（配列記法でDIサービスがある場合のみ）
                        // identifier（関数参照）の場合は$injectパターンで処理されるため、ここでは追加しない
                        let injected_services = self.collect_injected_services(second_arg, source);
                        if !injected_services.is_empty() {
                            if let Some((body_start, body_end)) = self.find_function_body_range(second_arg, source) {
                                let di_scope = DiScope {
                                    component_name: component_name.clone(),
                                    injected_services,
                                    body_start_line: body_start,
                                    body_end_line: body_end,
                                };
                                ctx.push_scope(di_scope);
                            }
                        }

                        if matches!(kind, SymbolKind::Service | SymbolKind::Factory) {
                            self.extract_service_methods(second_arg, source, uri, &component_name);
                        }

                        // 関数定義の位置を取得
                        if let Some((func_start, func_end)) = self.find_function_position(second_arg, source) {
                            // 関数定義の位置からJSDocを探す
                            (func_start, func_end, func_start.row)
                        } else {
                            // フォールバック: コンポーネント名の位置
                            (first_arg.start_position(), first_arg.end_position(), first_arg.start_position().row)
                        }
                    } else {
                        (first_arg.start_position(), first_arg.end_position(), first_arg.start_position().row)
                    };

                    // JSDocを探す（関数定義の位置またはコンポーネント名の位置から）
                    let docs = self.extract_jsdoc_for_line(docs_line, source);

                    let symbol = Symbol {
                        name: component_name.clone(),
                        kind,
                        uri: uri.clone(),
                        // 定義位置（ジャンプ先）
                        start_line: start.row as u32,
                        start_col: start.column as u32,
                        end_line: end.row as u32,
                        end_col: end.column as u32,
                        // シンボル名の位置（検索用）
                        name_start_line: name_start.row as u32,
                        name_start_col: name_start.column as u32,
                        name_end_line: name_end.row as u32,
                        name_end_col: name_end.column as u32,
                        docs,
                    };

                    self.index.add_definition(symbol);
                }
            }
        }
    }

    /// ノードから関数定義の位置を取得する
    ///
    /// - 配列の場合: 配列内の関数式を探す
    /// - 関数式の場合: その位置を返す
    /// - 識別子の場合: ファイル内の関数宣言/変数宣言を探す
    fn find_function_position(&self, node: Node, source: &str) -> Option<(tree_sitter::Point, tree_sitter::Point)> {
        match node.kind() {
            "function_expression" | "arrow_function" => {
                Some((node.start_position(), node.end_position()))
            }
            "array" => {
                // DI配列: ['$http', function($http) { ... }]
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "function_expression" || child.kind() == "arrow_function" {
                        return Some((child.start_position(), child.end_position()));
                    }
                }
                None
            }
            "identifier" => {
                // 変数参照: .factory('ExchangeService', ExchangeService)
                let func_name = self.node_text(node, source);
                // ルートノードを取得
                let root = {
                    let mut current = node;
                    while let Some(parent) = current.parent() {
                        current = parent;
                    }
                    current
                };
                // ファイル内の関数宣言または変数宣言を探す
                self.find_function_declaration_position(root, source, &func_name)
            }
            _ => None,
        }
    }

    /// ファイル内で指定された名前の関数宣言または変数宣言（関数式）を探す
    fn find_function_declaration_position(
        &self,
        node: Node,
        source: &str,
        name: &str,
    ) -> Option<(tree_sitter::Point, tree_sitter::Point)> {
        match node.kind() {
            "function_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    if self.node_text(name_node, source) == name {
                        return Some((node.start_position(), node.end_position()));
                    }
                }
            }
            "variable_declaration" | "lexical_declaration" => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "variable_declarator" {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            if self.node_text(name_node, source) == name {
                                if let Some(value_node) = child.child_by_field_name("value") {
                                    if value_node.kind() == "function_expression"
                                        || value_node.kind() == "arrow_function"
                                    {
                                        return Some((value_node.start_position(), value_node.end_position()));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        // 再帰的に子ノードを探索
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(pos) = self.find_function_declaration_position(child, source, name) {
                return Some(pos);
            }
        }

        None
    }

    /// サービス/ファクトリーの実装関数からメソッドを抽出する
    ///
    /// DI配列記法と直接関数渡しの両方に対応:
    /// ```javascript
    /// .service('Svc', ['$http', function($http) { ... }])
    /// .service('Svc', function() { ... })
    /// .factory('Svc', SvcFunction)  // 関数参照パターン
    /// ```
    fn extract_service_methods(&self, node: Node, source: &str, uri: &Url, service_name: &str) {
        if node.kind() == "array" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "function_expression" || child.kind() == "arrow_function" {
                    self.extract_methods_from_function(child, source, uri, service_name);
                }
            }
        } else if node.kind() == "function_expression" || node.kind() == "arrow_function" {
            self.extract_methods_from_function(node, source, uri, service_name);
        } else if node.kind() == "identifier" {
            // 関数参照パターン: .factory('Svc', SvcFunction)
            let func_name = self.node_text(node, source);
            // ルートから関数宣言を探す
            let root = node.parent().and_then(|n| {
                let mut current = n;
                while let Some(parent) = current.parent() {
                    current = parent;
                }
                Some(current)
            });
            if let Some(root) = root {
                if let Some(func_decl) = self.find_function_declaration(root, source, &func_name) {
                    self.extract_methods_from_function_decl(func_decl, source, uri, service_name);
                }
            }
        }
    }

    /// 関数宣言を探す
    fn find_function_declaration<'a>(&self, node: Node<'a>, source: &str, name: &str) -> Option<Node<'a>> {
        if node.kind() == "function_declaration" {
            if let Some(name_node) = node.child_by_field_name("name") {
                if self.node_text(name_node, source) == name {
                    return Some(node);
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = self.find_function_declaration(child, source, name) {
                return Some(found);
            }
        }
        None
    }

    /// 関数宣言からメソッド定義を抽出する
    fn extract_methods_from_function_decl(&self, func_node: Node, source: &str, uri: &Url, service_name: &str) {
        if let Some(body) = func_node.child_by_field_name("body") {
            // 内部の関数宣言を収集
            let mut func_decls: HashMap<String, LocalVarLocation> = HashMap::new();
            self.collect_function_declarations(body, source, &mut func_decls);

            // ローカル変数の定義位置も収集
            let mut local_vars: HashMap<String, LocalVarLocation> = HashMap::new();
            self.collect_local_vars(body, source, &mut local_vars);

            // 両方をマージ
            for (k, v) in func_decls {
                local_vars.insert(k, v);
            }

            // メソッド定義をスキャン
            self.scan_for_methods(body, source, uri, service_name, &local_vars);
        }
    }

    /// 関数宣言を収集する
    fn collect_function_declarations(&self, node: Node, source: &str, func_decls: &mut HashMap<String, LocalVarLocation>) {
        if node.kind() == "function_declaration" {
            if let Some(name_node) = node.child_by_field_name("name") {
                let func_name = self.node_text(name_node, source);
                // 関数名ではなく関数宣言全体の位置を記録する
                let start = node.start_position();
                let end = node.end_position();

                func_decls.insert(func_name, LocalVarLocation {
                    start_line: start.row as u32,
                    start_col: start.column as u32,
                    end_line: end.row as u32,
                    end_col: end.column as u32,
                });
            }
        }

        // 再帰的に子ノードを探索（ただし、関数の中には入らない）
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            // 関数の本体内部の関数宣言も収集
            self.collect_function_declarations(child, source, func_decls);
        }
    }

    /// 関数本体からメソッド定義を抽出する
    ///
    /// 1. ローカル変数・関数宣言の定義位置を収集
    /// 2. メソッド定義をスキャン（変数/関数参照の場合は実際の定義位置を使用）
    fn extract_methods_from_function(&self, func_node: Node, source: &str, uri: &Url, service_name: &str) {
        if let Some(body) = func_node.child_by_field_name("body") {
            // 内部の関数宣言を収集
            let mut func_decls: HashMap<String, LocalVarLocation> = HashMap::new();
            self.collect_function_declarations(body, source, &mut func_decls);

            // ローカル変数の定義位置を収集
            let mut local_vars: HashMap<String, LocalVarLocation> = HashMap::new();
            self.collect_local_vars(body, source, &mut local_vars);

            // 両方をマージ（関数宣言を優先）
            for (k, v) in func_decls {
                local_vars.insert(k, v);
            }

            // メソッド定義をスキャン
            self.scan_for_methods(body, source, uri, service_name, &local_vars);
        }
    }

    /// ローカル変数の定義位置を収集する
    ///
    /// 認識パターン:
    /// ```javascript
    /// var showNotify = function() { ... };
    /// var login = function(creds) { ... };
    /// ```
    fn collect_local_vars(&self, node: Node, source: &str, local_vars: &mut HashMap<String, LocalVarLocation>) {
        match node.kind() {
            "variable_declaration" | "lexical_declaration" => {
                // var/let/const 宣言
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "variable_declarator" {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            if let Some(value_node) = child.child_by_field_name("value") {
                                // 値が関数の場合のみ記録
                                // 変数名ではなく関数定義の位置を記録する
                                if value_node.kind() == "function_expression"
                                    || value_node.kind() == "arrow_function"
                                {
                                    let var_name = self.node_text(name_node, source);
                                    let start = value_node.start_position();
                                    let end = value_node.end_position();

                                    local_vars.insert(var_name, LocalVarLocation {
                                        start_line: start.row as u32,
                                        start_col: start.column as u32,
                                        end_line: end.row as u32,
                                        end_col: end.column as u32,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        // 再帰的に子ノードを探索
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.collect_local_vars(child, source, local_vars);
        }
    }

    /// 関数本体内をスキャンしてメソッド定義を探す
    ///
    /// 認識パターン:
    /// - `this.methodName = function() {}` - serviceパターン
    /// - `return { methodName: function() {} }` - factoryパターン
    /// - `return { methodName: varName }` - 変数参照パターン（実際の定義位置を使用）
    fn scan_for_methods(
        &self,
        node: Node,
        source: &str,
        uri: &Url,
        service_name: &str,
        local_vars: &HashMap<String, LocalVarLocation>,
    ) {
        match node.kind() {
            "expression_statement" => {
                if let Some(expr) = node.named_child(0) {
                    if expr.kind() == "assignment_expression" {
                        self.extract_this_method(expr, source, uri, service_name);
                    }
                }
            }
            "return_statement" => {
                if let Some(arg) = node.named_child(0) {
                    if arg.kind() == "object" {
                        self.extract_object_methods(arg, source, uri, service_name, local_vars);
                    }
                }
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.scan_for_methods(child, source, uri, service_name, local_vars);
        }
    }

    /// `this.methodName = function() {}` パターンからメソッドを抽出する
    ///
    /// serviceパターンで使用される:
    /// ```javascript
    /// .service('UserService', function() {
    ///     this.getAll = function() { ... };
    ///     this.getById = function(id) { ... };
    /// })
    /// ```
    ///
    /// `UserService.getAll`, `UserService.getById` として登録される
    fn extract_this_method(&self, assign_node: Node, source: &str, uri: &Url, service_name: &str) {
        if let Some(left) = assign_node.child_by_field_name("left") {
            if left.kind() == "member_expression" {
                if let Some(object) = left.child_by_field_name("object") {
                    if self.node_text(object, source) == "this" {
                        if let Some(property) = left.child_by_field_name("property") {
                            let method_name = self.node_text(property, source);
                            let start = property.start_position();
                            let end = property.end_position();

                            // 代入文の行からJSDocを探す
                            let docs = self.extract_jsdoc_for_line(assign_node.start_position().row, source);

                            let full_name = format!("{}.{}", service_name, method_name);
                            let symbol = Symbol {
                                name: full_name,
                                kind: SymbolKind::Method,
                                uri: uri.clone(),
                                // メソッドの場合、定義位置とシンボル名位置は同じ
                                start_line: start.row as u32,
                                start_col: start.column as u32,
                                end_line: end.row as u32,
                                end_col: end.column as u32,
                                name_start_line: start.row as u32,
                                name_start_col: start.column as u32,
                                name_end_line: end.row as u32,
                                name_end_col: end.column as u32,
                                docs,
                            };
                            self.index.add_definition(symbol);
                        }
                    }
                }
            }
        }
    }

    /// `return { ... }` オブジェクトからメソッドを抽出する
    ///
    /// factoryパターンで使用される:
    /// ```javascript
    /// .factory('AuthService', function() {
    ///     var login = function(creds) { ... };
    ///     return {
    ///         login: function(creds) { ... },  // 関数式 → この位置を登録
    ///         logout: logout,                   // 変数参照 → 変数定義位置を登録
    ///         isLoggedIn                        // shorthand → 変数定義位置を登録
    ///     };
    /// })
    /// ```
    ///
    /// `AuthService.login`, `AuthService.logout`, `AuthService.isLoggedIn` として登録
    fn extract_object_methods(
        &self,
        obj_node: Node,
        source: &str,
        uri: &Url,
        service_name: &str,
        local_vars: &HashMap<String, LocalVarLocation>,
    ) {
        let mut cursor = obj_node.walk();
        for child in obj_node.children(&mut cursor) {
            match child.kind() {
                "pair" => {
                    if let Some(key) = child.child_by_field_name("key") {
                        if let Some(value) = child.child_by_field_name("value") {
                            let method_name = self.node_text(key, source);
                            let full_name = format!("{}.{}", service_name, method_name);
                            // シンボル名の位置はキーの位置
                            let name_start = key.start_position();
                            let name_end = key.end_position();

                            match value.kind() {
                                // 直接関数定義: login: function() {}
                                "function_expression" | "arrow_function" => {
                                    let start = key.start_position();
                                    let end = key.end_position();
                                    // pairノードの行からJSDocを探す
                                    let docs = self.extract_jsdoc_for_line(child.start_position().row, source);
                                    let symbol = Symbol {
                                        name: full_name,
                                        kind: SymbolKind::Method,
                                        uri: uri.clone(),
                                        start_line: start.row as u32,
                                        start_col: start.column as u32,
                                        end_line: end.row as u32,
                                        end_col: end.column as u32,
                                        name_start_line: name_start.row as u32,
                                        name_start_col: name_start.column as u32,
                                        name_end_line: name_end.row as u32,
                                        name_end_col: name_end.column as u32,
                                        docs,
                                    };
                                    self.index.add_definition(symbol);
                                }
                                // 変数参照: showNotify: showNotify
                                "identifier" => {
                                    let var_name = self.node_text(value, source);
                                    // ローカル変数の定義位置があればそれを使用
                                    if let Some(loc) = local_vars.get(&var_name) {
                                        // ローカル変数の定義位置からJSDocを探す
                                        let docs = self.extract_jsdoc_for_line(loc.start_line as usize, source);
                                        let symbol = Symbol {
                                            name: full_name,
                                            kind: SymbolKind::Method,
                                            uri: uri.clone(),
                                            // 定義位置はローカル変数の位置
                                            start_line: loc.start_line,
                                            start_col: loc.start_col,
                                            end_line: loc.end_line,
                                            end_col: loc.end_col,
                                            // シンボル名の位置はキーの位置
                                            name_start_line: name_start.row as u32,
                                            name_start_col: name_start.column as u32,
                                            name_end_line: name_end.row as u32,
                                            name_end_col: name_end.column as u32,
                                            docs,
                                        };
                                        self.index.add_definition(symbol);
                                    } else {
                                        // ローカル変数が見つからない場合はキーの位置を使用
                                        let start = key.start_position();
                                        let end = key.end_position();
                                        let docs = self.extract_jsdoc_for_line(child.start_position().row, source);
                                        let symbol = Symbol {
                                            name: full_name,
                                            kind: SymbolKind::Method,
                                            uri: uri.clone(),
                                            start_line: start.row as u32,
                                            start_col: start.column as u32,
                                            end_line: end.row as u32,
                                            end_col: end.column as u32,
                                            name_start_line: name_start.row as u32,
                                            name_start_col: name_start.column as u32,
                                            name_end_line: name_end.row as u32,
                                            name_end_col: name_end.column as u32,
                                            docs,
                                        };
                                        self.index.add_definition(symbol);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                // shorthand: { showNotify } (ES6)
                "shorthand_property_identifier" => {
                    let method_name = self.node_text(child, source);
                    let full_name = format!("{}.{}", service_name, method_name);
                    // シンボル名の位置はshorthandプロパティの位置
                    let name_start = child.start_position();
                    let name_end = child.end_position();

                    // ローカル変数の定義位置があればそれを使用
                    if let Some(loc) = local_vars.get(&method_name) {
                        // ローカル変数の定義位置からJSDocを探す
                        let docs = self.extract_jsdoc_for_line(loc.start_line as usize, source);
                        let symbol = Symbol {
                            name: full_name,
                            kind: SymbolKind::Method,
                            uri: uri.clone(),
                            // 定義位置はローカル変数の位置
                            start_line: loc.start_line,
                            start_col: loc.start_col,
                            end_line: loc.end_line,
                            end_col: loc.end_col,
                            // シンボル名の位置
                            name_start_line: name_start.row as u32,
                            name_start_col: name_start.column as u32,
                            name_end_line: name_end.row as u32,
                            name_end_col: name_end.column as u32,
                            docs,
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
                            start_line: start.row as u32,
                            start_col: start.column as u32,
                            end_line: end.row as u32,
                            end_col: end.column as u32,
                            name_start_line: name_start.row as u32,
                            name_start_col: name_start.column as u32,
                            name_end_line: name_end.row as u32,
                            name_end_col: name_end.column as u32,
                            docs,
                        };
                        self.index.add_definition(symbol);
                    }
                }
                _ => {}
            }
        }
    }

    /// DI配列から依存サービスを参照として抽出する
    ///
    /// 認識パターン:
    /// ```javascript
    /// .controller('Ctrl', ['$scope', 'UserService', function(...) {}])
    /// ```
    ///
    /// `$` で始まるAngular組み込みサービスはスキップ
    fn extract_dependencies(&self, node: Node, source: &str, uri: &Url) {
        if node.kind() == "array" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "string" {
                    let dep_name = self.extract_string_value(child, source);
                    if !dep_name.starts_with('$') {
                        let start = child.start_position();
                        let end = child.end_position();

                        let reference = SymbolReference {
                            name: dep_name,
                            uri: uri.clone(),
                            start_line: start.row as u32,
                            start_col: start.column as u32,
                            end_line: end.row as u32,
                            end_col: end.column as u32,
                        };

                        self.index.add_reference(reference);
                    }
                }
            }
        }
    }

    /// 識別子を解析し、既知の定義への参照として登録する
    ///
    /// インデックスに存在するシンボル名と一致する識別子を参照として登録
    /// 短すぎる識別子やキーワードはスキップ
    /// DIされていないサービスへの参照は登録しない
    fn analyze_identifier(&self, node: Node, source: &str, uri: &Url, ctx: &AnalyzerContext) {
        let name = self.node_text(node, source);

        if name.len() < 2 || is_common_keyword(&name) {
            return;
        }

        if self.index.has_definition(&name) {
            // DIチェック: このスコープでサービスがDIされているか確認
            let current_line = node.start_position().row as u32;
            if !ctx.is_injected_at(&name, current_line) {
                return;
            }

            let start = node.start_position();
            let end = node.end_position();

            let reference = SymbolReference {
                name,
                uri: uri.clone(),
                start_line: start.row as u32,
                start_col: start.column as u32,
                end_line: end.row as u32,
                end_col: end.column as u32,
            };

            self.index.add_reference(reference);
        }
    }

    /// ASTノードからソーステキストを取得する
    fn node_text(&self, node: Node, source: &str) -> String {
        source[node.byte_range()].to_string()
    }

    /// 文字列ノードから値を取得する（クォートを除去）
    fn extract_string_value(&self, node: Node, source: &str) -> String {
        let text = self.node_text(node, source);
        text.trim_matches(|c| c == '"' || c == '\'').to_string()
    }

    /// 指定された行の直前にあるJSDocコメントを抽出する
    ///
    /// チェーン呼び出しの場合でもコンポーネント名の位置から正しくJSDocを検出する
    fn extract_jsdoc_for_line(&self, target_line: usize, source: &str) -> Option<String> {
        let lines: Vec<&str> = source.lines().collect();

        // 対象行の直前の行から上に向かってJSDocコメントを探す
        // 空行はスキップする
        let mut search_line = target_line as i32 - 1;
        let mut jsdoc_end_line: Option<usize> = None;

        // まず、JSDocコメントの終了行（*/）を探す
        while search_line >= 0 {
            let line = lines.get(search_line as usize).map(|s| s.trim()).unwrap_or("");

            if line.is_empty() {
                search_line -= 1;
                continue;
            }

            if line.ends_with("*/") {
                jsdoc_end_line = Some(search_line as usize);
                break;
            }

            // 空行でもコメント終了でもない場合は、JSDocはない
            break;
        }

        let end_line = jsdoc_end_line?;

        // JSDocコメントの開始行（/**）を探す
        search_line = end_line as i32;
        while search_line >= 0 {
            let line = lines.get(search_line as usize).map(|s| s.trim()).unwrap_or("");

            if line.starts_with("/**") {
                // JSDocコメントを構築
                let jsdoc_lines: Vec<&str> = lines[search_line as usize..=end_line].to_vec();
                let jsdoc_text = jsdoc_lines.join("\n");
                return Some(self.parse_jsdoc(&jsdoc_text));
            }

            search_line -= 1;
        }

        None
    }

    /// JSDocコメントをパースして整形する
    fn parse_jsdoc(&self, comment: &str) -> String {
        comment
            .lines()
            .map(|line| {
                let trimmed = line.trim();
                // 各行から /** */ * を除去
                let trimmed = trimmed.trim_start_matches("/**").trim_end_matches("*/");
                trimmed.trim_start_matches('*').trim()
            })
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// DI配列から依存サービス名（$以外）を収集する
    ///
    /// 認識パターン:
    /// ```javascript
    /// ['$scope', 'UserService', function($scope, UserService) {}]
    /// ```
    fn collect_injected_services(&self, node: Node, source: &str) -> Vec<String> {
        let mut services = Vec::new();

        if node.kind() == "array" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "string" {
                    let dep_name = self.extract_string_value(child, source);
                    if !dep_name.starts_with('$') {
                        services.push(dep_name);
                    }
                }
            }
        }

        services
    }

    /// 関数本体の行範囲を取得する
    ///
    /// DI配列または関数式から関数本体の開始行と終了行を抽出
    fn find_function_body_range(&self, node: Node, source: &str) -> Option<(u32, u32)> {
        let func_node = match node.kind() {
            "array" => {
                let mut cursor = node.walk();
                node.children(&mut cursor)
                    .find(|c| c.kind() == "function_expression" || c.kind() == "arrow_function")
            }
            "function_expression" | "arrow_function" => Some(node),
            "identifier" => {
                // 変数参照の場合は関数宣言を探す
                let func_name = self.node_text(node, source);
                let root = {
                    let mut current = node;
                    while let Some(parent) = current.parent() {
                        current = parent;
                    }
                    current
                };
                self.find_function_declaration(root, source, &func_name)
            }
            _ => None,
        }?;

        if let Some(body) = func_node.child_by_field_name("body") {
            return Some((body.start_position().row as u32, body.end_position().row as u32));
        }
        None
    }

    /// $inject パターン用に関数宣言を収集する
    ///
    /// ファイル内の全関数宣言を収集し、関数名と本体の範囲を記録
    fn collect_function_declarations_for_inject(&self, node: Node, source: &str, ctx: &mut AnalyzerContext) {
        if node.kind() == "function_declaration" {
            if let Some(name_node) = node.child_by_field_name("name") {
                let func_name = self.node_text(name_node, source);
                if let Some(body) = node.child_by_field_name("body") {
                    let start = body.start_position().row as u32;
                    let end = body.end_position().row as u32;
                    ctx.function_ranges.insert(func_name, (start, end));
                }
            }
        }

        // 子ノードを再帰的に走査
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.collect_function_declarations_for_inject(child, source, ctx);
        }
    }

    /// $inject パターンを収集する
    ///
    /// 認識パターン:
    /// ```javascript
    /// MyController.$inject = ['$scope', 'UserService'];
    /// ```
    fn collect_inject_patterns(&self, node: Node, source: &str, ctx: &mut AnalyzerContext) {
        if node.kind() == "expression_statement" {
            if let Some(expr) = node.named_child(0) {
                if expr.kind() == "assignment_expression" {
                    if let Some(left) = expr.child_by_field_name("left") {
                        if left.kind() == "member_expression" {
                            if let Some(object) = left.child_by_field_name("object") {
                                if let Some(property) = left.child_by_field_name("property") {
                                    let prop_name = self.node_text(property, source);
                                    if prop_name == "$inject" {
                                        let func_name = self.node_text(object, source);
                                        if let Some(right) = expr.child_by_field_name("right") {
                                            let services = self.collect_injected_services(right, source);
                                            if !services.is_empty() {
                                                ctx.inject_map.insert(func_name, services);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // 子ノードを再帰的に走査
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.collect_inject_patterns(child, source, ctx);
        }
    }
}

/// JavaScriptの予約語・キーワードかどうかを判定する
fn is_common_keyword(name: &str) -> bool {
    matches!(
        name,
        "function" | "var" | "let" | "const" | "if" | "else" | "for" | "while"
            | "return" | "true" | "false" | "null" | "undefined" | "this"
            | "new" | "typeof" | "instanceof" | "in" | "of"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::SymbolIndex;

    #[test]
    fn test_di_check_with_di() {
        // DIされている場合は参照が登録される
        let source = r#"
angular.module('app')
.service('MyService', function() {
    this.doSomething = function() {};
})
.controller('TestCtrl', ['$scope', 'MyService', function($scope, MyService) {
    MyService.doSomething();
}]);
"#;
        let index = Arc::new(SymbolIndex::new());
        let analyzer = AngularJsAnalyzer::new(index.clone());
        let uri = Url::parse("file:///test.js").unwrap();

        // Pass 1: definitions
        analyzer.analyze_document_with_options(&uri, source, true);
        // Pass 2: references
        analyzer.analyze_document_with_options(&uri, source, false);

        // MyService.doSomething への参照が登録されているはず
        let refs = index.get_references("MyService.doSomething");
        assert!(!refs.is_empty(), "DIされている場合は参照が登録されるべき");
    }

    #[test]
    fn test_di_check_without_di() {
        // DIされていない場合は参照が登録されない
        let source = r#"
angular.module('app')
.service('MyService', function() {
    this.doSomething = function() {};
})
.controller('TestCtrl', ['$scope', function($scope) {
    MyService.doSomething();
}]);
"#;
        let index = Arc::new(SymbolIndex::new());
        let analyzer = AngularJsAnalyzer::new(index.clone());
        let uri = Url::parse("file:///test.js").unwrap();

        // Pass 1: definitions
        analyzer.analyze_document_with_options(&uri, source, true);
        // Pass 2: references
        analyzer.analyze_document_with_options(&uri, source, false);

        // MyService.doSomething への参照が登録されていないはず
        let refs = index.get_references("MyService.doSomething");
        assert!(refs.is_empty(), "DIされていない場合は参照が登録されないべき");
    }

    #[test]
    fn test_di_check_inject_pattern() {
        // $inject パターンでDIされている場合は参照が登録される
        let source = r#"
angular.module('app')
.service('MyService', function() {
    this.doSomething = function() {};
});

function TestController($scope, MyService) {
    MyService.doSomething();
}
TestController.$inject = ['$scope', 'MyService'];
"#;
        let index = Arc::new(SymbolIndex::new());
        let analyzer = AngularJsAnalyzer::new(index.clone());
        let uri = Url::parse("file:///test.js").unwrap();

        // Pass 1: definitions
        analyzer.analyze_document_with_options(&uri, source, true);
        // Pass 2: references
        analyzer.analyze_document_with_options(&uri, source, false);

        // MyService.doSomething への参照が登録されているはず
        let refs = index.get_references("MyService.doSomething");
        assert!(!refs.is_empty(), "$injectパターンでDIされている場合は参照が登録されるべき");
    }

    #[test]
    fn test_di_check_inject_pattern_without_di() {
        // $inject パターンでDIされていない場合は参照が登録されない
        let source = r#"
angular.module('app')
.service('MyService', function() {
    this.doSomething = function() {};
});

function TestController($scope) {
    MyService.doSomething();
}
TestController.$inject = ['$scope'];
"#;
        let index = Arc::new(SymbolIndex::new());
        let analyzer = AngularJsAnalyzer::new(index.clone());
        let uri = Url::parse("file:///test.js").unwrap();

        // Pass 1: definitions
        analyzer.analyze_document_with_options(&uri, source, true);
        // Pass 2: references
        analyzer.analyze_document_with_options(&uri, source, false);

        // MyService.doSomething への参照が登録されていないはず
        let refs = index.get_references("MyService.doSomething");
        assert!(refs.is_empty(), "$injectパターンでDIされていない場合は参照が登録されないべき");
    }

    #[test]
    fn test_di_check_iife_inject_pattern() {
        // IIFE内の$injectパターンでDIされている場合は参照が登録される
        let source = r#"
angular.module('app')
.service('notifyService', function() {
    this.showNotify = function() {};
});

(function() {
    'use strict';
    angular
        .module('app')
        .controller('TestController', TestController);

    TestController.$inject = ['notifyService'];

    function TestController(notifyService) {
        notifyService.showNotify();
    }
})();
"#;
        let mut parser = super::super::JsParser::new();
        let tree = parser.parse(source).unwrap();
        let mut ctx = AnalyzerContext::new();
        let index = Arc::new(SymbolIndex::new());
        let analyzer = AngularJsAnalyzer::new(index.clone());

        analyzer.collect_function_declarations_for_inject(tree.root_node(), source, &mut ctx);
        analyzer.collect_inject_patterns(tree.root_node(), source, &mut ctx);

        let uri = Url::parse("file:///test.js").unwrap();
        // Pass 1: definitions
        analyzer.analyze_document_with_options(&uri, source, true);
        // Pass 2: references
        analyzer.analyze_document_with_options(&uri, source, false);

        // notifyService.showNotify への参照が登録されているはず
        let refs = index.get_references("notifyService.showNotify");
        assert!(!refs.is_empty(), "IIFE内の$injectパターンでDIされている場合は参照が登録されるべき: refs={:?}", refs);
    }

    #[test]
    fn test_collect_inject_patterns() {
        // $inject パターンが正しく収集されているか確認
        let source = r#"
(function() {
    TestController.$inject = ['notifyService'];

    function TestController(notifyService) {
        notifyService.showNotify();
    }
})();
"#;
        let mut parser = super::super::JsParser::new();
        let tree = parser.parse(source).unwrap();
        let mut ctx = AnalyzerContext::new();

        let index = Arc::new(SymbolIndex::new());
        let analyzer = AngularJsAnalyzer::new(index);

        analyzer.collect_function_declarations_for_inject(tree.root_node(), source, &mut ctx);
        analyzer.collect_inject_patterns(tree.root_node(), source, &mut ctx);

        assert!(ctx.function_ranges.contains_key("TestController"), "TestController should be in function_ranges");
        assert!(ctx.inject_map.contains_key("TestController"), "TestController should be in inject_map");

        // is_injected_at のテスト
        // 行5 (0-indexed: 4) は関数本体内
        assert!(ctx.is_injected_at("notifyService", 5), "notifyService should be injected at line 5");
        assert!(!ctx.is_injected_at("otherService", 5), "otherService should NOT be injected at line 5");
    }

    #[test]
    fn test_is_injected_at_with_inject_pattern() {
        // is_injected_at が $inject パターンで正しく動作するか確認
        let mut ctx = AnalyzerContext::new();
        ctx.function_ranges.insert("TestController".to_string(), (4, 6));
        ctx.inject_map.insert("TestController".to_string(), vec!["notifyService".to_string()]);

        // 行5は関数本体内 (4 <= 5 <= 6)
        assert!(ctx.is_injected_at("notifyService", 5), "notifyService should be injected at line 5");
        // 行3は関数本体外 (3 < 4)
        assert!(!ctx.is_injected_at("notifyService", 3), "notifyService should NOT be injected at line 3");
        // 存在しないサービス
        assert!(!ctx.is_injected_at("otherService", 5), "otherService should NOT be injected");
    }
}
