use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::context::{AnalyzerContext, DiScope};
use super::AngularJsAnalyzer;
use crate::index::{BindingSource, ControllerScope, Symbol, SymbolKind, TemplateBinding};

impl AngularJsAnalyzer {
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
    /// - `$uibModal.open({...})` - モーダルバインディング
    pub(super) fn analyze_call_expression(&self, node: Node, source: &str, uri: &Url, ctx: &mut AnalyzerContext) {
        if let Some(callee) = node.child_by_field_name("function") {
            let callee_text = self.node_text(callee, source);

            if callee_text == "angular.module" {
                self.extract_module_definition(node, source, uri, ctx);
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
                        "open" => self.extract_modal_binding(node, callee, source),
                        "config" | "run" => self.extract_run_config_di(node, source, ctx),
                        "when" | "otherwise" => self.extract_route_when_di(node, source, uri, ctx),
                        _ => {}
                    }
                }
            }
        }
    }

    /// $uibModal.open() / $modal.open() からテンプレートバインディングを抽出
    fn extract_modal_binding(&self, node: Node, callee: Node, source: &str) {
        // オブジェクトが$uibModalや$modalかチェック
        if let Some(object) = callee.child_by_field_name("object") {
            let obj_text = self.node_text(object, source);
            if !obj_text.ends_with("Modal") && !obj_text.ends_with("$uibModal") && !obj_text.ends_with("$modal") {
                return;
            }
        } else {
            return;
        }

        // 引数からオブジェクトを取得
        if let Some(args) = node.child_by_field_name("arguments") {
            if let Some(first_arg) = args.named_child(0) {
                if first_arg.kind() == "object" {
                    self.extract_template_binding_from_object(first_arg, source, BindingSource::UibModal);
                }
            }
        }
    }

    /// JSオブジェクトからcontrollerとtemplateUrlを抽出してバインディングを登録
    fn extract_template_binding_from_object(&self, obj_node: Node, source: &str, binding_source: BindingSource) {
        let mut controller_name: Option<String> = None;
        let mut template_url: Option<String> = None;

        let mut cursor = obj_node.walk();
        for child in obj_node.children(&mut cursor) {
            if child.kind() == "pair" {
                if let Some(key) = child.child_by_field_name("key") {
                    let key_name = self.node_text(key, source);
                    // 識別子の場合はそのまま、文字列の場合はクォートを除去
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

        // 両方揃っていればバインディングを登録
        if let (Some(controller), Some(template)) = (controller_name, template_url) {
            let binding = TemplateBinding {
                template_path: template,
                controller_name: controller,
                source: binding_source,
            };
            self.index.add_template_binding(binding);
        }
    }

    /// `$routeProvider.when()` または `.otherwise()` のcontrollerプロパティからDIスコープを抽出する
    ///
    /// 認識パターン:
    /// ```javascript
    /// $routeProvider.when('/path', {
    ///     templateUrl: 'template.html',
    ///     controller: function($scope) { $scope.foo = 1; }
    /// })
    /// $routeProvider.when('/path', {
    ///     templateUrl: 'template.html',
    ///     controller: ['$scope', function($scope) { $scope.foo = 1; }]
    /// })
    /// ```
    fn extract_route_when_di(&self, node: Node, source: &str, uri: &Url, ctx: &mut AnalyzerContext) {
        if let Some(args) = node.child_by_field_name("arguments") {
            // when('/path', {config}) の場合、設定オブジェクトは第2引数
            // otherwise({config}) の場合、設定オブジェクトは第1引数
            let config_arg = args.named_child(1).or_else(|| args.named_child(0));

            if let Some(config_obj) = config_arg {
                if config_obj.kind() == "object" {
                    self.extract_controller_di_from_config_object(config_obj, source, uri, ctx);
                }
            }
        }
    }

    /// 設定オブジェクトからcontrollerプロパティを探し、DIスコープを抽出する
    /// また、controller と templateUrl の組み合わせでテンプレートバインディングも登録する
    fn extract_controller_di_from_config_object(&self, obj_node: Node, source: &str, uri: &Url, ctx: &mut AnalyzerContext) {
        // まずテンプレートバインディング用にcontrollerとtemplateUrlを収集
        self.extract_template_binding_from_object(obj_node, source, BindingSource::RouteProvider);

        let mut cursor = obj_node.walk();
        for child in obj_node.children(&mut cursor) {
            if child.kind() == "pair" {
                if let Some(key) = child.child_by_field_name("key") {
                    let key_name = self.node_text(key, source);
                    let key_name = key_name.trim_matches(|c| c == '"' || c == '\'');

                    if key_name == "controller" {
                        if let Some(value) = child.child_by_field_name("value") {
                            // controller: function($scope) {} パターン
                            // controller: ['$scope', function($scope) {}] パターン
                            // controller: 'ControllerName' パターン（文字列の場合は参照を登録）
                            // controller: ControllerName パターン（識別子の場合は関数参照を探す）
                            let (injected_services, has_scope, has_root_scope) = if value.kind() == "array" {
                                self.extract_dependencies(value, source, uri);
                                (self.collect_injected_services(value, source),
                                 self.has_scope_in_di_array(value, source),
                                 self.has_root_scope_in_di_array(value, source))
                            } else if value.kind() == "function_expression" || value.kind() == "arrow_function" {
                                (self.collect_services_from_function_params(value, source),
                                 self.has_scope_in_function_params(value, source),
                                 self.has_root_scope_in_function_params(value, source))
                            } else if value.kind() == "identifier" {
                                (self.collect_services_from_function_ref(value, source),
                                 self.has_scope_in_function_ref(value, source),
                                 self.has_root_scope_in_function_ref(value, source))
                            } else if value.kind() == "string" {
                                // controller: 'ControllerName' パターン
                                // コントローラー名への参照を登録
                                let controller_name = self.extract_string_value(value, source);
                                let start = value.start_position();
                                let end = value.end_position();
                                let reference = crate::index::SymbolReference {
                                    name: controller_name,
                                    uri: uri.clone(),
                                    start_line: self.offset_line(start.row as u32),
                                    start_col: start.column as u32,
                                    end_line: self.offset_line(end.row as u32),
                                    end_col: end.column as u32,
                                };
                                self.index.add_reference(reference);
                                // 文字列パターンではDIスコープは抽出しない
                                (Vec::new(), false, false)
                            } else {
                                (Vec::new(), false, false)
                            };

                            // DIサービスがあるか、$scopeまたは$rootScopeがある場合はスコープを追加
                            if !injected_services.is_empty() || has_scope || has_root_scope {
                                if let Some((body_start, body_end)) = self.find_function_body_range(value, source) {
                                    // $scopeがある場合はControllerScopeも登録
                                    if has_scope {
                                        self.index.add_controller_scope(ControllerScope {
                                            name: "route".to_string(),
                                            uri: uri.clone(),
                                            start_line: body_start,
                                            end_line: body_end,
                                            injected_services: injected_services.clone(),
                                        });
                                    }

                                    let di_scope = DiScope {
                                        component_name: "route".to_string(),
                                        injected_services,
                                        body_start_line: body_start,
                                        body_end_line: body_end,
                                        has_scope,
                                        has_root_scope,
                                    };
                                    ctx.push_scope(di_scope);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// `.run()` または `.config()` のDIスコープを抽出する
    ///
    /// これらはシンボル定義を作成しないが、DIスコープを作成して
    /// $rootScope などの解析を可能にする
    fn extract_run_config_di(&self, node: Node, source: &str, ctx: &mut AnalyzerContext) {
        if let Some(args) = node.child_by_field_name("arguments") {
            if let Some(first_arg) = args.named_child(0) {
                let (injected_services, has_scope, has_root_scope) = if first_arg.kind() == "array" {
                    // 配列記法: ['$rootScope', function($rootScope) {}]
                    (self.collect_injected_services(first_arg, source),
                     self.has_scope_in_di_array(first_arg, source),
                     self.has_root_scope_in_di_array(first_arg, source))
                } else if first_arg.kind() == "function_expression" || first_arg.kind() == "arrow_function" {
                    // 直接関数記法: function($rootScope) {}
                    (self.collect_services_from_function_params(first_arg, source),
                     self.has_scope_in_function_params(first_arg, source),
                     self.has_root_scope_in_function_params(first_arg, source))
                } else if first_arg.kind() == "identifier" {
                    // 関数参照: .run(AppInit)
                    (self.collect_services_from_function_ref(first_arg, source),
                     self.has_scope_in_function_ref(first_arg, source),
                     self.has_root_scope_in_function_ref(first_arg, source))
                } else {
                    (Vec::new(), false, false)
                };

                // DIサービスがあるか、$scopeまたは$rootScopeがある場合はスコープを追加
                if !injected_services.is_empty() || has_scope || has_root_scope {
                    if let Some((body_start, body_end)) = self.find_function_body_range(first_arg, source) {
                        let di_scope = DiScope {
                            component_name: "run".to_string(), // run/config には名前がない
                            injected_services,
                            body_start_line: body_start,
                            body_end_line: body_end,
                            has_scope,
                            has_root_scope,
                        };
                        ctx.push_scope(di_scope);
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
    pub(super) fn extract_module_definition(&self, node: Node, source: &str, uri: &Url, ctx: &mut AnalyzerContext) {
        if let Some(args) = node.child_by_field_name("arguments") {
            if let Some(first_arg) = args.named_child(0) {
                if first_arg.kind() == "string" {
                    let name = self.extract_string_value(first_arg, source);
                    let start = first_arg.start_position();
                    let end = first_arg.end_position();

                    // 現在のモジュール名をコンテキストに設定
                    ctx.set_current_module(name.clone());

                    let docs = self.extract_jsdoc_for_line(start.row, source);
                    let symbol = Symbol {
                        name: name.clone(),
                        kind: SymbolKind::Module,
                        uri: uri.clone(),
                        // 定義位置とシンボル名位置は同じ（文字列リテラル）
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
    pub(super) fn extract_component_definition(&self, node: Node, source: &str, uri: &Url, kind: SymbolKind, ctx: &mut AnalyzerContext) {
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

                        // DIスコープを追加
                        // 配列記法、直接関数記法、関数参照の全パターンをサポート
                        let (injected_services, has_scope, has_root_scope) = if second_arg.kind() == "array" {
                            // 配列記法: ['$scope', 'Service', function($scope, Service) {}]
                            (self.collect_injected_services(second_arg, source),
                             self.has_scope_in_di_array(second_arg, source),
                             self.has_root_scope_in_di_array(second_arg, source))
                        } else if second_arg.kind() == "function_expression" || second_arg.kind() == "arrow_function" {
                            // 直接関数記法: function($scope, Service) {}
                            (self.collect_services_from_function_params(second_arg, source),
                             self.has_scope_in_function_params(second_arg, source),
                             self.has_root_scope_in_function_params(second_arg, source))
                        } else if second_arg.kind() == "identifier" {
                            // 関数参照: .controller('Ctrl', MyController)
                            // $inject がある場合は別途処理されるが、ない場合はここで関数宣言のパラメータを解析
                            (self.collect_services_from_function_ref(second_arg, source),
                             self.has_scope_in_function_ref(second_arg, source),
                             self.has_root_scope_in_function_ref(second_arg, source))
                        } else {
                            (Vec::new(), false, false)
                        };

                        // DIサービスがあるか、$scopeまたは$rootScopeがある場合はスコープを追加
                        if !injected_services.is_empty() || has_scope || has_root_scope {
                            if let Some((body_start, body_end)) = self.find_function_body_range(second_arg, source) {
                                // コントローラーの場合はスコープ情報を SymbolIndex に登録
                                if kind == SymbolKind::Controller {
                                    self.index.add_controller_scope(ControllerScope {
                                        name: component_name.clone(),
                                        uri: uri.clone(),
                                        start_line: body_start,
                                        end_line: body_end,
                                        injected_services: injected_services.clone(),
                                    });
                                }

                                let di_scope = DiScope {
                                    component_name: component_name.clone(),
                                    injected_services,
                                    body_start_line: body_start,
                                    body_end_line: body_end,
                                    has_scope,
                                    has_root_scope,
                                };
                                ctx.push_scope(di_scope);
                            }
                        }

                        // Service/Factoryの場合はメソッドを抽出
                        if matches!(kind, SymbolKind::Service | SymbolKind::Factory) {
                            self.extract_service_methods(second_arg, source, uri, &component_name);
                        }

                        // Controllerの場合もthis.methodパターンを抽出
                        // controller as構文でalias.methodとしてアクセスされる
                        if kind == SymbolKind::Controller {
                            self.extract_controller_methods(second_arg, source, uri, &component_name);
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
                        start_line: self.offset_line(start.row as u32),
                        start_col: start.column as u32,
                        end_line: self.offset_line(end.row as u32),
                        end_col: end.column as u32,
                        // シンボル名の位置（検索用）
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
        }
    }

    /// ノードから関数定義の位置を取得する
    ///
    /// - 配列の場合: 配列内の関数式を探す
    /// - 関数式の場合: その位置を返す
    /// - 識別子の場合: ファイル内の関数宣言/変数宣言を探す
    pub(super) fn find_function_position(&self, node: Node, source: &str) -> Option<(tree_sitter::Point, tree_sitter::Point)> {
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
    pub(super) fn find_function_declaration_position(
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
}
