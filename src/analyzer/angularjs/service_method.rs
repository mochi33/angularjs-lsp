use std::collections::HashMap;

use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::context::LocalVarLocation;
use super::AngularJsAnalyzer;
use crate::index::{Symbol, SymbolKind};

impl AngularJsAnalyzer {
    /// サービス/ファクトリーの実装関数からメソッドを抽出する
    ///
    /// DI配列記法と直接関数渡し、ES6 classの全パターンに対応:
    /// ```javascript
    /// .service('Svc', ['$http', function($http) { ... }])
    /// .service('Svc', function() { ... })
    /// .factory('Svc', SvcFunction)  // 関数参照パターン
    /// .service('Svc', class { getData() { ... } })  // ES6 class式
    /// .service('Svc', MyServiceClass)  // ES6 class参照
    /// ```
    pub(super) fn extract_service_methods(&self, node: Node, source: &str, uri: &Url, service_name: &str) {
        if node.kind() == "array" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "function_expression" || child.kind() == "arrow_function" {
                    self.extract_methods_from_function(child, source, uri, service_name);
                } else if child.kind() == "class" {
                    // ES6 class式: ['$http', class { ... }]
                    self.extract_methods_from_class(child, source, uri, service_name);
                }
            }
        } else if node.kind() == "function_expression" || node.kind() == "arrow_function" {
            self.extract_methods_from_function(node, source, uri, service_name);
        } else if node.kind() == "class" {
            // ES6 class式: .service('Svc', class { ... })
            self.extract_methods_from_class(node, source, uri, service_name);
        } else if node.kind() == "identifier" {
            // 関数参照またはclass参照パターン: .factory('Svc', SvcFunction) or .service('Svc', SvcClass)
            let ref_name = self.node_text(node, source);
            // ルートから関数宣言またはclass宣言を探す
            let root = node.parent().and_then(|n| {
                let mut current = n;
                while let Some(parent) = current.parent() {
                    current = parent;
                }
                Some(current)
            });
            if let Some(root) = root {
                // まず関数宣言を探す
                if let Some(func_decl) = self.find_function_declaration(root, source, &ref_name) {
                    self.extract_methods_from_function_decl(func_decl, source, uri, service_name);
                } else if let Some(class_decl) = self.find_class_declaration(root, source, &ref_name) {
                    // class宣言を探す
                    self.extract_methods_from_class(class_decl, source, uri, service_name);
                }
            }
        }
    }

    /// ES6 classからService/Factory/Controllerのメソッドを抽出する
    ///
    /// 認識パターン:
    /// ```javascript
    /// class MyService {
    ///     constructor($http) { ... }
    ///     getData() { return this.http.get('/api'); }
    ///     postData(data) { return this.http.post('/api', data); }
    /// }
    /// ```
    ///
    /// `MyService.getData`, `MyService.postData` として登録（constructorは除外）
    fn extract_methods_from_class(&self, class_node: Node, source: &str, uri: &Url, service_name: &str) {
        // class_body を取得
        if let Some(body) = class_node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                if child.kind() == "method_definition" {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let method_name = self.node_text(name_node, source);

                        // constructorはスキップ（DIエントリポイントであってメソッドではない）
                        if method_name == "constructor" {
                            continue;
                        }

                        let start = name_node.start_position();
                        let end = name_node.end_position();

                        // JSDocを抽出
                        let docs = self.extract_jsdoc_for_line(child.start_position().row, source);

                        // パラメータを抽出
                        let parameters = child.child_by_field_name("parameters")
                            .and_then(|params| self.extract_params_from_node(params, source));

                        let full_name = format!("{}.{}", service_name, method_name);
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

    /// パラメータノードからパラメータ名のリストを抽出する
    fn extract_params_from_node(&self, params_node: Node, source: &str) -> Option<Vec<String>> {
        let mut params = Vec::new();
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            if child.kind() == "identifier" {
                params.push(self.node_text(child, source));
            }
        }
        if params.is_empty() {
            None
        } else {
            Some(params)
        }
    }

    /// 関数宣言を探す
    pub(super) fn find_function_declaration<'a>(&self, node: Node<'a>, source: &str, name: &str) -> Option<Node<'a>> {
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
                    start_line: self.offset_line(start.row as u32),
                    start_col: start.column as u32,
                    end_line: self.offset_line(end.row as u32),
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
                                        start_line: self.offset_line(start.row as u32),
                                        start_col: start.column as u32,
                                        end_line: self.offset_line(end.row as u32),
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

                            // 右辺からパラメータを抽出
                            let parameters = assign_node
                                .child_by_field_name("right")
                                .and_then(|right| self.extract_function_params(right, source));

                            let full_name = format!("{}.{}", service_name, method_name);
                            let symbol = Symbol {
                                name: full_name,
                                kind: SymbolKind::Method,
                                uri: uri.clone(),
                                // メソッドの場合、定義位置とシンボル名位置は同じ
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
                                    // パラメータを抽出
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
                                            name_start_line: self.offset_line(name_start.row as u32),
                                            name_start_col: name_start.column as u32,
                                            name_end_line: self.offset_line(name_end.row as u32),
                                            name_end_col: name_end.column as u32,
                                            docs,
                                            parameters: None,
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

    /// コントローラーの実装関数からthis.methodパターンを抽出する
    ///
    /// controller as構文で使用される:
    /// ```javascript
    /// .controller('FormCustomItemController', function() {
    ///     this.onChangeText = function(item) { ... };
    /// })
    /// ```
    ///
    /// ES6 classの場合はclassメソッドを抽出:
    /// ```javascript
    /// class FormController {
    ///     submit() { ... }
    /// }
    /// .controller('FormController', FormController)
    /// ```
    ///
    /// `FormCustomItemController.onChangeText` / `FormController.submit` として登録される
    /// (Service/Factoryと同じ形式で、HTML側でalias.methodとしてアクセス可能)
    pub(super) fn extract_controller_methods(&self, node: Node, source: &str, uri: &Url, controller_name: &str) {
        if node.kind() == "array" {
            // DI配列記法: ['$scope', function($scope) { ... }] または ['$scope', class { ... }]
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "function_expression" || child.kind() == "arrow_function" {
                    self.extract_this_methods_from_controller(child, source, uri, controller_name);
                } else if child.kind() == "class" {
                    // ES6 class式: ['$scope', class { submit() {} }]
                    self.extract_methods_from_class(child, source, uri, controller_name);
                }
            }
        } else if node.kind() == "function_expression" || node.kind() == "arrow_function" {
            // 直接関数記法
            self.extract_this_methods_from_controller(node, source, uri, controller_name);
        } else if node.kind() == "class" {
            // ES6 class式: .controller('Ctrl', class { ... })
            self.extract_methods_from_class(node, source, uri, controller_name);
        } else if node.kind() == "identifier" {
            // 関数参照またはclass参照パターン: .controller('Ctrl', MyController)
            let ref_name = self.node_text(node, source);
            let root = node.parent().and_then(|n| {
                let mut current = n;
                while let Some(parent) = current.parent() {
                    current = parent;
                }
                Some(current)
            });
            if let Some(root) = root {
                // まず関数宣言を探す
                if let Some(func_decl) = self.find_function_declaration(root, source, &ref_name) {
                    if let Some(body) = func_decl.child_by_field_name("body") {
                        self.scan_for_this_methods(body, source, uri, controller_name);
                    }
                } else if let Some(class_decl) = self.find_class_declaration(root, source, &ref_name) {
                    // class宣言を探す
                    self.extract_methods_from_class(class_decl, source, uri, controller_name);
                }
            }
        }
    }

    /// コントローラー関数本体からthis.methodを抽出
    fn extract_this_methods_from_controller(&self, func_node: Node, source: &str, uri: &Url, controller_name: &str) {
        if let Some(body) = func_node.child_by_field_name("body") {
            // まず `var vm = this;` のようなthisエイリアスを収集
            let this_aliases = self.collect_this_aliases(body, source);
            // this.method と vm.method の両方をスキャン
            self.scan_for_this_methods_with_aliases(body, source, uri, controller_name, &this_aliases);
        }
    }

    /// `var vm = this;` のようなthisエイリアスを収集
    ///
    /// 認識パターン:
    /// ```javascript
    /// var vm = this;
    /// const vm = this;
    /// let vm = this;
    /// ```
    fn collect_this_aliases(&self, node: Node, source: &str) -> Vec<String> {
        let mut aliases = Vec::new();
        self.collect_this_aliases_recursive(node, source, &mut aliases);
        aliases
    }

    fn collect_this_aliases_recursive(&self, node: Node, source: &str, aliases: &mut Vec<String>) {
        match node.kind() {
            "variable_declaration" | "lexical_declaration" => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "variable_declarator" {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            if let Some(value_node) = child.child_by_field_name("value") {
                                // `var vm = this;` パターンをチェック
                                if value_node.kind() == "this" {
                                    let var_name = self.node_text(name_node, source);
                                    aliases.push(var_name);
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
            self.collect_this_aliases_recursive(child, source, aliases);
        }
    }

    /// 関数本体内をスキャンしてthis.methodとvm.method定義を探す
    fn scan_for_this_methods_with_aliases(
        &self,
        node: Node,
        source: &str,
        uri: &Url,
        controller_name: &str,
        this_aliases: &[String],
    ) {
        if node.kind() == "expression_statement" {
            if let Some(expr) = node.named_child(0) {
                if expr.kind() == "assignment_expression" {
                    self.extract_this_or_alias_method(expr, source, uri, controller_name, this_aliases);
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.scan_for_this_methods_with_aliases(child, source, uri, controller_name, this_aliases);
        }
    }

    /// `this.method = ...` または `vm.method = ...` パターンからメソッドを抽出
    fn extract_this_or_alias_method(
        &self,
        assign_node: Node,
        source: &str,
        uri: &Url,
        controller_name: &str,
        this_aliases: &[String],
    ) {
        if let Some(left) = assign_node.child_by_field_name("left") {
            if left.kind() == "member_expression" {
                if let Some(object) = left.child_by_field_name("object") {
                    let obj_text = self.node_text(object, source);

                    // `this` または thisエイリアス（vm等）かどうかをチェック
                    let is_this_or_alias = obj_text == "this" || this_aliases.contains(&obj_text);

                    if is_this_or_alias {
                        if let Some(property) = left.child_by_field_name("property") {
                            let method_name = self.node_text(property, source);
                            let start = property.start_position();
                            let end = property.end_position();

                            let docs = self.extract_jsdoc_for_line(assign_node.start_position().row, source);

                            // 右辺からパラメータを抽出
                            let parameters = assign_node
                                .child_by_field_name("right")
                                .and_then(|right| self.extract_function_params(right, source));

                            let full_name = format!("{}.{}", controller_name, method_name);
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

    /// 関数本体内をスキャンしてthis.method定義を探す（後方互換性のため残す）
    fn scan_for_this_methods(&self, node: Node, source: &str, uri: &Url, controller_name: &str) {
        // thisエイリアスを収集して使用
        let this_aliases = self.collect_this_aliases(node, source);
        self.scan_for_this_methods_with_aliases(node, source, uri, controller_name, &this_aliases);
    }
}
