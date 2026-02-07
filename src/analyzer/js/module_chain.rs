use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::context::AnalyzerContext;
use super::AngularJsAnalyzer;
use crate::model::ControllerScope;

impl AngularJsAnalyzer {
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
                                                    self.index.controllers.add_controller_scope(ControllerScope {
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
    pub(super) fn find_variable_value_for_di<'a>(&self, root: Node<'a>, source: &str, var_name: &str) -> Option<Node<'a>> {
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
