use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::context::AnalyzerContext;
use super::AngularJsAnalyzer;
use crate::index::{ControllerScope, SymbolReference};

impl AngularJsAnalyzer {
    /// ES6 classノードからconstructorメソッドを取得する
    ///
    /// class_declaration と class (class式) の両方に対応
    ///
    /// 認識パターン:
    /// ```javascript
    /// class MyController {
    ///     constructor($scope, Service) { ... }
    /// }
    /// ```
    pub(super) fn get_constructor_from_class<'a>(&self, class_node: Node<'a>, source: &str) -> Option<Node<'a>> {
        // class_body を取得
        let body = class_node.child_by_field_name("body")?;

        // class_body 内の method_definition を探してconstructorを見つける
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "method_definition" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = self.node_text(name_node, source);
                    if name == "constructor" {
                        return Some(child);
                    }
                }
            }
        }
        None
    }

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
    /// class MyController { constructor($scope, $http) {} }
    /// ```
    pub(super) fn has_scope_in_function_params(&self, node: Node, source: &str) -> bool {
        let func_node = match node.kind() {
            "function_expression" | "arrow_function" | "function_declaration" | "method_definition" => Some(node),
            // ES6 class の場合はconstructorを取得
            "class_declaration" | "class" => self.get_constructor_from_class(node, source),
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
    /// class AppController { constructor($rootScope) {} }
    /// ```
    pub(super) fn has_root_scope_in_function_params(&self, node: Node, source: &str) -> bool {
        let func_node = match node.kind() {
            "function_expression" | "arrow_function" | "function_declaration" | "method_definition" => Some(node),
            // ES6 class の場合はconstructorを取得
            "class_declaration" | "class" => self.get_constructor_from_class(node, source),
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
    /// class MyController { constructor($scope, MyService) {} }
    /// ```
    pub(super) fn collect_services_from_function_params(&self, node: Node, source: &str) -> Vec<String> {
        let mut services = Vec::new();

        let func_node = match node.kind() {
            "function_expression" | "arrow_function" | "function_declaration" | "method_definition" => Some(node),
            // ES6 class の場合はconstructorを取得
            "class_declaration" | "class" => self.get_constructor_from_class(node, source),
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

    /// 関数参照またはclass参照（identifier）から関数宣言/class宣言を探し、パラメータに $scope があるかチェック
    ///
    /// 関数参照パターン用:
    /// ```javascript
    /// .controller('Ctrl', MyController);
    /// function MyController($scope, Service) {}
    /// class MyController { constructor($scope, Service) {} }
    /// ```
    pub(super) fn has_scope_in_function_ref(&self, node: Node, source: &str) -> bool {
        if node.kind() != "identifier" {
            return false;
        }

        let ref_name = self.node_text(node, source);
        let root = {
            let mut current = node;
            while let Some(parent) = current.parent() {
                current = parent;
            }
            current
        };

        // まず関数宣言を探す
        if let Some(func_decl) = self.find_function_declaration(root, source, &ref_name) {
            return self.has_scope_in_function_params(func_decl, source);
        }
        // 次にclass宣言を探す
        if let Some(class_decl) = self.find_class_declaration(root, source, &ref_name) {
            return self.has_scope_in_function_params(class_decl, source);
        }
        false
    }

    /// 関数参照またはclass参照（identifier）から関数宣言/class宣言を探し、パラメータに $rootScope があるかチェック
    ///
    /// 関数参照パターン用:
    /// ```javascript
    /// .run(AppInit);
    /// function AppInit($rootScope) {}
    /// class AppInit { constructor($rootScope) {} }
    /// ```
    pub(super) fn has_root_scope_in_function_ref(&self, node: Node, source: &str) -> bool {
        if node.kind() != "identifier" {
            return false;
        }

        let ref_name = self.node_text(node, source);
        let root = {
            let mut current = node;
            while let Some(parent) = current.parent() {
                current = parent;
            }
            current
        };

        // まず関数宣言を探す
        if let Some(func_decl) = self.find_function_declaration(root, source, &ref_name) {
            return self.has_root_scope_in_function_params(func_decl, source);
        }
        // 次にclass宣言を探す
        if let Some(class_decl) = self.find_class_declaration(root, source, &ref_name) {
            return self.has_root_scope_in_function_params(class_decl, source);
        }
        false
    }

    /// 関数参照またはclass参照（identifier）から関数宣言/class宣言を探し、パラメータからサービスを収集
    ///
    /// 関数参照パターン用:
    /// ```javascript
    /// .controller('Ctrl', MyController);
    /// function MyController($scope, Service) {}
    /// class MyController { constructor($scope, Service) {} }
    /// ```
    pub(super) fn collect_services_from_function_ref(&self, node: Node, source: &str) -> Vec<String> {
        if node.kind() != "identifier" {
            return Vec::new();
        }

        let ref_name = self.node_text(node, source);
        let root = {
            let mut current = node;
            while let Some(parent) = current.parent() {
                current = parent;
            }
            current
        };

        // まず関数宣言を探す
        if let Some(func_decl) = self.find_function_declaration(root, source, &ref_name) {
            return self.collect_services_from_function_params(func_decl, source);
        }
        // 次にclass宣言を探す
        if let Some(class_decl) = self.find_class_declaration(root, source, &ref_name) {
            return self.collect_services_from_function_params(class_decl, source);
        }
        Vec::new()
    }

    /// 関数本体の行範囲を取得する
    ///
    /// DI配列または関数式から関数本体の開始行と終了行を抽出
    /// ES6 classの場合はconstructor本体の範囲を返す
    pub(super) fn find_function_body_range(&self, node: Node, source: &str) -> Option<(u32, u32)> {
        let func_node = match node.kind() {
            "array" => {
                let mut cursor = node.walk();
                node.children(&mut cursor)
                    .find(|c| c.kind() == "function_expression" || c.kind() == "arrow_function" || c.kind() == "class")
                    .and_then(|n| {
                        if n.kind() == "class" {
                            // class式の場合はconstructorを取得
                            self.get_constructor_from_class(n, source)
                        } else {
                            Some(n)
                        }
                    })
            }
            "function_expression" | "arrow_function" => Some(node),
            // ES6 class の場合はconstructorを取得
            "class_declaration" | "class" => self.get_constructor_from_class(node, source),
            "identifier" => {
                // 変数参照の場合は関数宣言またはclass宣言を探す
                let func_name = self.node_text(node, source);
                let root = {
                    let mut current = node;
                    while let Some(parent) = current.parent() {
                        current = parent;
                    }
                    current
                };
                // まず関数宣言を探す
                if let Some(func_decl) = self.find_function_declaration(root, source, &func_name) {
                    Some(func_decl)
                } else {
                    // 次にclass宣言を探してconstructorを返す
                    self.find_class_declaration(root, source, &func_name)
                        .and_then(|class_decl| self.get_constructor_from_class(class_decl, source))
                }
            }
            _ => None,
        }?;

        if let Some(body) = func_node.child_by_field_name("body") {
            return Some((body.start_position().row as u32, body.end_position().row as u32));
        }
        None
    }

    /// AST内でclass宣言を名前で検索する
    ///
    /// 認識パターン:
    /// ```javascript
    /// class MyController { ... }
    /// ```
    pub(super) fn find_class_declaration<'a>(&self, node: Node<'a>, source: &str, name: &str) -> Option<Node<'a>> {
        if node.kind() == "class_declaration" {
            if let Some(name_node) = node.child_by_field_name("name") {
                if self.node_text(name_node, source) == name {
                    return Some(node);
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = self.find_class_declaration(child, source, name) {
                return Some(found);
            }
        }
        None
    }

    /// $inject パターン用に関数宣言およびclass宣言を収集する
    ///
    /// ファイル内の全関数宣言とclass宣言を収集し、名前と本体の範囲を記録
    /// classの場合はconstructor本体の範囲を記録
    pub(super) fn collect_function_declarations_for_inject(&self, node: Node, source: &str, ctx: &mut AnalyzerContext) {
        match node.kind() {
            "function_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let func_name = self.node_text(name_node, source);
                    if let Some(body) = node.child_by_field_name("body") {
                        let start = body.start_position().row as u32;
                        let end = body.end_position().row as u32;
                        ctx.function_ranges.insert(func_name, (start, end));
                    }
                }
            }
            "class_declaration" => {
                // class宣言の場合はconstructor本体の範囲を記録
                if let Some(name_node) = node.child_by_field_name("name") {
                    let class_name = self.node_text(name_node, source);
                    if let Some(constructor) = self.get_constructor_from_class(node, source) {
                        if let Some(body) = constructor.child_by_field_name("body") {
                            let start = body.start_position().row as u32;
                            let end = body.end_position().row as u32;
                            ctx.function_ranges.insert(class_name, (start, end));
                        }
                    }
                }
            }
            _ => {}
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

    /// 関数/class参照パターンのコンポーネント登録を事前収集する
    ///
    /// 認識パターン:
    /// ```javascript
    /// angular.module('app').controller('MyController', MyController);
    /// // MyController は関数宣言またはclass宣言
    /// ```
    ///
    /// $inject パターンがない関数/class参照でも $scope 追跡を可能にするため、
    /// コンポーネント登録時に関数/class本体の範囲と$scope情報を事前収集する
    pub(super) fn collect_component_ref_scopes(&self, node: Node, source: &str, uri: &Url, ctx: &mut AnalyzerContext) {
        // .controller(), .service(), .factory() 呼び出しを探す
        if node.kind() == "call_expression" {
            if let Some(callee) = node.child_by_field_name("function") {
                if callee.kind() == "member_expression" {
                    if let Some(property) = callee.child_by_field_name("property") {
                        let method_name = self.node_text(property, source);
                        if matches!(method_name.as_str(), "controller" | "service" | "factory") {
                            // 引数を取得
                            if let Some(args) = node.child_by_field_name("arguments") {
                                // 第2引数がidentifierの場合
                                if let Some(second_arg) = args.named_child(1) {
                                    if second_arg.kind() == "identifier" {
                                        let ref_name = self.node_text(second_arg, source);

                                        // 既に $inject パターンで登録済みならスキップ
                                        if ctx.inject_has_scope.contains_key(&ref_name) {
                                            return;
                                        }

                                        // ルートノードを取得
                                        let root = {
                                            let mut current = node;
                                            while let Some(parent) = current.parent() {
                                                current = parent;
                                            }
                                            current
                                        };

                                        // 関数宣言またはclass宣言を探す
                                        let (body_range, has_scope, has_root_scope, services) =
                                            if let Some(func_decl) = self.find_function_declaration(root, source, &ref_name) {
                                                let has_scope = self.has_scope_in_function_params(func_decl, source);
                                                let has_root_scope = self.has_root_scope_in_function_params(func_decl, source);
                                                let services = self.collect_services_from_function_params(func_decl, source);
                                                let body_range = func_decl.child_by_field_name("body")
                                                    .map(|body| (body.start_position().row as u32, body.end_position().row as u32));
                                                (body_range, has_scope, has_root_scope, services)
                                            } else if let Some(class_decl) = self.find_class_declaration(root, source, &ref_name) {
                                                let has_scope = self.has_scope_in_function_params(class_decl, source);
                                                let has_root_scope = self.has_root_scope_in_function_params(class_decl, source);
                                                let services = self.collect_services_from_function_params(class_decl, source);
                                                let body_range = self.get_constructor_from_class(class_decl, source)
                                                    .and_then(|constructor| constructor.child_by_field_name("body"))
                                                    .map(|body| (body.start_position().row as u32, body.end_position().row as u32));
                                                (body_range, has_scope, has_root_scope, services)
                                            } else {
                                                (None, false, false, Vec::new())
                                            };

                                        // スコープ情報を登録
                                        if let Some((start_line, end_line)) = body_range {
                                            if has_scope || has_root_scope || !services.is_empty() {
                                                // ControllerScope を登録（$scope がDIされている場合）
                                                if has_scope && method_name == "controller" {
                                                    self.index.add_controller_scope(ControllerScope {
                                                        name: ref_name.clone(),
                                                        uri: uri.clone(),
                                                        start_line,
                                                        end_line,
                                                        injected_services: services.clone(),
                                                    });
                                                }

                                                // $inject パターンと同じ形式で登録
                                                ctx.function_ranges.insert(ref_name.clone(), (start_line, end_line));
                                                ctx.inject_map.insert(ref_name.clone(), services);
                                                ctx.inject_has_scope.insert(ref_name.clone(), has_scope);
                                                ctx.inject_has_root_scope.insert(ref_name, has_root_scope);
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

        // export default ['dep1', 'dep2', FunctionRef] パターンを処理
        // export default variableName パターン（変数が配列を参照している場合）も処理
        if node.kind() == "export_statement" {
            // "export default" かどうかを確認
            let has_default = node.children(&mut node.walk()).any(|c| c.kind() == "default");
            if has_default {
                // ルートノードを取得
                let root = {
                    let mut current = node;
                    while let Some(parent) = current.parent() {
                        current = parent;
                    }
                    current
                };

                // エクスポートされる値を取得（直接配列、または識別子経由で配列）
                let array_node = if let Some(array) = node.children(&mut node.walk()).find(|c| c.kind() == "array") {
                    Some(array)
                } else if let Some(ident) = node.children(&mut node.walk()).find(|c| c.kind() == "identifier") {
                    // 識別子の場合、変数の値を探す
                    let ident_name = self.node_text(ident, source);
                    self.find_variable_value_for_di(root, source, &ident_name)
                        .filter(|n| n.kind() == "array")
                } else {
                    None
                };

                if let Some(array) = array_node {
                    // DI配列パターンかチェック
                    let children: Vec<_> = array.named_children(&mut array.walk()).collect();
                    if !children.is_empty() {
                        let last = children.last().unwrap();
                        let is_function_like = matches!(
                            last.kind(),
                            "function_expression" | "arrow_function" | "identifier" | "class"
                        );
                        // 最後の要素以外が全て文字列であること
                        let is_di_array = is_function_like &&
                            children[..children.len() - 1].iter().all(|c| c.kind() == "string");

                        if is_di_array && last.kind() == "identifier" {
                            let ref_name = self.node_text(*last, source);

                            // 依存関係を抽出
                            let dependencies: Vec<String> = children[..children.len() - 1]
                                .iter()
                                .filter(|c| c.kind() == "string")
                                .map(|c| self.extract_string_value(*c, source))
                                .collect();

                            // Angular以外の依存（サービス）を抽出
                            let services: Vec<String> = dependencies
                                .iter()
                                .filter(|d| !d.starts_with('$'))
                                .cloned()
                                .collect();

                            let has_scope = dependencies.iter().any(|d| d == "$scope");
                            let has_root_scope = dependencies.iter().any(|d| d == "$rootScope");

                            // 関数宣言またはclass宣言を探す
                            let body_range = if let Some(func_decl) = self.find_function_declaration(root, source, &ref_name) {
                                func_decl.child_by_field_name("body")
                                    .map(|body| (body.start_position().row as u32, body.end_position().row as u32))
                            } else if let Some(class_decl) = self.find_class_declaration(root, source, &ref_name) {
                                self.get_constructor_from_class(class_decl, source)
                                    .and_then(|constructor| constructor.child_by_field_name("body"))
                                    .map(|body| (body.start_position().row as u32, body.end_position().row as u32))
                            } else {
                                None
                            };

                            // スコープ情報を登録
                            if let Some((start_line, end_line)) = body_range {
                                if has_scope || has_root_scope || !services.is_empty() {
                                    // $inject パターンと同じ形式で登録
                                    ctx.function_ranges.insert(ref_name.clone(), (start_line, end_line));
                                    ctx.inject_map.insert(ref_name.clone(), services);
                                    ctx.inject_has_scope.insert(ref_name.clone(), has_scope);
                                    ctx.inject_has_root_scope.insert(ref_name, has_root_scope);
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
            self.collect_component_ref_scopes(child, source, uri, ctx);
        }
    }

    /// 指定された名前の変数宣言を探し、その値ノードを返す（DI事前収集用）
    fn find_variable_value_for_di<'a>(&self, root: Node<'a>, source: &str, var_name: &str) -> Option<Node<'a>> {
        self.find_variable_value_for_di_recursive(root, source, var_name)
    }

    fn find_variable_value_for_di_recursive<'a>(&self, node: Node<'a>, source: &str, var_name: &str) -> Option<Node<'a>> {
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
            if let Some(found) = self.find_variable_value_for_di_recursive(child, source, var_name) {
                return Some(found);
            }
        }

        None
    }
}
