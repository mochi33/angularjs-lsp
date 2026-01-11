use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::context::AnalyzerContext;
use super::AngularJsAnalyzer;
use crate::index::{ControllerScope, SymbolReference};

impl AngularJsAnalyzer {
    /// `$inject` パターンを解析する
    ///
    /// 認識パターン:
    /// ```javascript
    /// MyController.$inject = ['$scope', 'MyService'];
    /// ```
    pub(super) fn analyze_inject_pattern(&self, node: Node, source: &str, uri: &Url) {
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
    pub(super) fn extract_inject_dependencies(&self, node: Node, source: &str, uri: &Url) {
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
                            start_line: self.offset_line(start.row as u32),
                            start_col: start.column as u32,
                            end_line: self.offset_line(end.row as u32),
                            end_col: end.column as u32,
                        };

                        self.index.add_reference(reference);
                    }
                }
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
    pub(super) fn extract_dependencies(&self, node: Node, source: &str, uri: &Url) {
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
                            start_line: self.offset_line(start.row as u32),
                            start_col: start.column as u32,
                            end_line: self.offset_line(end.row as u32),
                            end_col: end.column as u32,
                        };

                        self.index.add_reference(reference);
                    }
                }
            }
        }
    }

    /// DI配列から依存サービス名（$以外）を収集する
    ///
    /// 認識パターン:
    /// ```javascript
    /// ['$scope', 'UserService', function($scope, UserService) {}]
    /// ```
    pub(super) fn collect_injected_services(&self, node: Node, source: &str) -> Vec<String> {
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

    /// DI配列に $scope が含まれているかチェックする
    pub(super) fn has_scope_in_di_array(&self, node: Node, source: &str) -> bool {
        if node.kind() == "array" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "string" {
                    let dep_name = self.extract_string_value(child, source);
                    if dep_name == "$scope" {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// DI配列に $rootScope が含まれているかチェックする
    pub(super) fn has_root_scope_in_di_array(&self, node: Node, source: &str) -> bool {
        if node.kind() == "array" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "string" {
                    let dep_name = self.extract_string_value(child, source);
                    if dep_name == "$rootScope" {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// 関数パラメータに $scope が含まれているかチェックする
    ///
    /// 直接関数パターン用:
    /// ```javascript
    /// .controller('Ctrl', function($scope, $http) {})
    /// function MyController($scope, $http) {}
    /// ```
    pub(super) fn has_scope_in_function_params(&self, node: Node, source: &str) -> bool {
        let func_node = match node.kind() {
            "function_expression" | "arrow_function" | "function_declaration" => Some(node),
            _ => None,
        };

        if let Some(func) = func_node {
            if let Some(params) = func.child_by_field_name("parameters") {
                let mut cursor = params.walk();
                for child in params.children(&mut cursor) {
                    if child.kind() == "identifier" {
                        let param_name = self.node_text(child, source);
                        if param_name == "$scope" {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// 関数パラメータに $rootScope が含まれているかチェックする
    ///
    /// 直接関数パターン用:
    /// ```javascript
    /// .run(function($rootScope) {})
    /// function AppController($rootScope) {}
    /// ```
    pub(super) fn has_root_scope_in_function_params(&self, node: Node, source: &str) -> bool {
        let func_node = match node.kind() {
            "function_expression" | "arrow_function" | "function_declaration" => Some(node),
            _ => None,
        };

        if let Some(func) = func_node {
            if let Some(params) = func.child_by_field_name("parameters") {
                let mut cursor = params.walk();
                for child in params.children(&mut cursor) {
                    if child.kind() == "identifier" {
                        let param_name = self.node_text(child, source);
                        if param_name == "$rootScope" {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// 関数パラメータから $scope 以外のサービス名を収集する
    ///
    /// 直接関数パターン用:
    /// ```javascript
    /// .controller('Ctrl', function($scope, MyService) {})
    /// function MyController($scope, MyService) {}
    /// ```
    pub(super) fn collect_services_from_function_params(&self, node: Node, source: &str) -> Vec<String> {
        let mut services = Vec::new();

        let func_node = match node.kind() {
            "function_expression" | "arrow_function" | "function_declaration" => Some(node),
            _ => None,
        };

        if let Some(func) = func_node {
            if let Some(params) = func.child_by_field_name("parameters") {
                let mut cursor = params.walk();
                for child in params.children(&mut cursor) {
                    if child.kind() == "identifier" {
                        let param_name = self.node_text(child, source);
                        // $で始まらないパラメータをサービスとして収集
                        if !param_name.starts_with('$') {
                            services.push(param_name);
                        }
                    }
                }
            }
        }

        services
    }

    /// 関数参照（identifier）から関数宣言を探し、パラメータに $scope があるかチェック
    ///
    /// 関数参照パターン用:
    /// ```javascript
    /// .controller('Ctrl', MyController);
    /// function MyController($scope, Service) {}
    /// ```
    pub(super) fn has_scope_in_function_ref(&self, node: Node, source: &str) -> bool {
        if node.kind() != "identifier" {
            return false;
        }

        let func_name = self.node_text(node, source);
        let root = {
            let mut current = node;
            while let Some(parent) = current.parent() {
                current = parent;
            }
            current
        };

        if let Some(func_decl) = self.find_function_declaration(root, source, &func_name) {
            return self.has_scope_in_function_params(func_decl, source);
        }
        false
    }

    /// 関数参照（identifier）から関数宣言を探し、パラメータに $rootScope があるかチェック
    ///
    /// 関数参照パターン用:
    /// ```javascript
    /// .run(AppInit);
    /// function AppInit($rootScope) {}
    /// ```
    pub(super) fn has_root_scope_in_function_ref(&self, node: Node, source: &str) -> bool {
        if node.kind() != "identifier" {
            return false;
        }

        let func_name = self.node_text(node, source);
        let root = {
            let mut current = node;
            while let Some(parent) = current.parent() {
                current = parent;
            }
            current
        };

        if let Some(func_decl) = self.find_function_declaration(root, source, &func_name) {
            return self.has_root_scope_in_function_params(func_decl, source);
        }
        false
    }

    /// 関数参照（identifier）から関数宣言を探し、パラメータからサービスを収集
    ///
    /// 関数参照パターン用:
    /// ```javascript
    /// .controller('Ctrl', MyController);
    /// function MyController($scope, Service) {}
    /// ```
    pub(super) fn collect_services_from_function_ref(&self, node: Node, source: &str) -> Vec<String> {
        if node.kind() != "identifier" {
            return Vec::new();
        }

        let func_name = self.node_text(node, source);
        let root = {
            let mut current = node;
            while let Some(parent) = current.parent() {
                current = parent;
            }
            current
        };

        if let Some(func_decl) = self.find_function_declaration(root, source, &func_name) {
            return self.collect_services_from_function_params(func_decl, source);
        }
        Vec::new()
    }

    /// 関数本体の行範囲を取得する
    ///
    /// DI配列または関数式から関数本体の開始行と終了行を抽出
    pub(super) fn find_function_body_range(&self, node: Node, source: &str) -> Option<(u32, u32)> {
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
    pub(super) fn collect_function_declarations_for_inject(&self, node: Node, source: &str, ctx: &mut AnalyzerContext) {
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
    pub(super) fn collect_inject_patterns(&self, node: Node, source: &str, uri: &Url, ctx: &mut AnalyzerContext) {
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
                                            let has_scope = self.has_scope_in_di_array(right, source);
                                            let has_root_scope = self.has_root_scope_in_di_array(right, source);
                                            // サービスまたは$scopeまたは$rootScopeがある場合は記録
                                            if !services.is_empty() || has_scope || has_root_scope {
                                                // $scope がDIされている場合、ControllerScope を登録
                                                if has_scope {
                                                    if let Some((start_line, end_line)) = ctx.function_ranges.get(&func_name) {
                                                        self.index.add_controller_scope(ControllerScope {
                                                            name: func_name.clone(),
                                                            uri: uri.clone(),
                                                            start_line: *start_line,
                                                            end_line: *end_line,
                                                            injected_services: services.clone(),
                                                        });
                                                    }
                                                }

                                                ctx.inject_map.insert(func_name.clone(), services);
                                                ctx.inject_has_scope.insert(func_name.clone(), has_scope);
                                                ctx.inject_has_root_scope.insert(func_name, has_root_scope);
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
            self.collect_inject_patterns(child, source, uri, ctx);
        }
    }
}
