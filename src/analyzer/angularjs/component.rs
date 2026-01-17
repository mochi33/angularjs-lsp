use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::context::{AnalyzerContext, DiScope};
use super::AngularJsAnalyzer;
use crate::index::{BindingSource, ComponentTemplateUrl, ControllerScope, Symbol, SymbolKind, TemplateBinding};

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
                        "component" => self.extract_angular_component(node, source, uri, ctx),
                        "provider" => self.extract_component_definition(node, source, uri, SymbolKind::Provider, ctx),
                        "filter" => self.extract_component_definition(node, source, uri, SymbolKind::Filter, ctx),
                        "constant" => self.extract_component_definition(node, source, uri, SymbolKind::Constant, ctx),
                        "value" => self.extract_component_definition(node, source, uri, SymbolKind::Value, ctx),
                        "open" => self.extract_modal_binding(node, callee, source, uri),
                        "config" | "run" => self.extract_run_config_di(node, source, ctx),
                        "when" | "otherwise" => self.extract_route_when_di(node, source, uri, ctx),
                        _ => {}
                    }
                }
            }
        }
    }

    /// $uibModal.open() / $modal.open() からテンプレートバインディングを抽出
    fn extract_modal_binding(&self, node: Node, callee: Node, source: &str, uri: &Url) {
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
                    self.extract_template_binding_from_object(first_arg, source, uri, BindingSource::UibModal);
                }
            }
        }
    }

    /// JSオブジェクトからcontrollerとtemplateUrlを抽出してバインディングを登録
    fn extract_template_binding_from_object(&self, obj_node: Node, source: &str, uri: &Url, binding_source: BindingSource) {
        let mut controller_name: Option<String> = None;
        let mut template_url: Option<String> = None;
        let mut template_url_line: Option<u32> = None;

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
                                    template_url_line = Some(self.offset_line(value.start_position().row as u32));
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
                binding_uri: uri.clone(),
                binding_line: template_url_line.unwrap_or(self.offset_line(obj_node.start_position().row as u32)),
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
        self.extract_template_binding_from_object(obj_node, source, uri, BindingSource::RouteProvider);

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
                        // 配列記法、直接関数記法、class式、関数参照の全パターンをサポート
                        let (injected_services, has_scope, has_root_scope) = if second_arg.kind() == "array" {
                            // 配列記法: ['$scope', 'Service', function($scope, Service) {}]
                            // または: ['$scope', 'Service', class { constructor($scope, Service) {} }]
                            (self.collect_injected_services(second_arg, source),
                             self.has_scope_in_di_array(second_arg, source),
                             self.has_root_scope_in_di_array(second_arg, source))
                        } else if second_arg.kind() == "function_expression" || second_arg.kind() == "arrow_function" {
                            // 直接関数記法: function($scope, Service) {}
                            (self.collect_services_from_function_params(second_arg, source),
                             self.has_scope_in_function_params(second_arg, source),
                             self.has_root_scope_in_function_params(second_arg, source))
                        } else if second_arg.kind() == "class" {
                            // ES6 class式: class { constructor($scope, Service) {} }
                            (self.collect_services_from_function_params(second_arg, source),
                             self.has_scope_in_function_params(second_arg, source),
                             self.has_root_scope_in_function_params(second_arg, source))
                        } else if second_arg.kind() == "identifier" {
                            // 関数参照またはclass参照: .controller('Ctrl', MyController)
                            // $inject がある場合は別途処理されるが、ない場合はここで関数宣言/class宣言のパラメータを解析
                            (self.collect_services_from_function_ref(second_arg, source),
                             self.has_scope_in_function_ref(second_arg, source),
                             self.has_root_scope_in_function_ref(second_arg, source))
                        } else {
                            (Vec::new(), false, false)
                        };

                        // DIサービスがあるか、$scopeまたは$rootScopeがある場合はスコープを追加
                        if !injected_services.is_empty() || has_scope || has_root_scope {
                            if let Some((body_start, body_end)) = self.find_function_body_range(second_arg, source) {
                                // Controller/Service/Factory の場合はスコープ情報を SymbolIndex に登録
                                // これにより補完時にInjectされたサービスを優先表示できる
                                if matches!(kind, SymbolKind::Controller | SymbolKind::Service | SymbolKind::Factory) {
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
    /// - 配列の場合: 配列内の関数式またはclass式を探す
    /// - 関数式の場合: その位置を返す
    /// - class式の場合: constructorの位置またはclass全体の位置を返す
    /// - 識別子の場合: ファイル内の関数宣言/変数宣言/class宣言を探す
    pub(super) fn find_function_position(&self, node: Node, source: &str) -> Option<(tree_sitter::Point, tree_sitter::Point)> {
        match node.kind() {
            "function_expression" | "arrow_function" => {
                Some((node.start_position(), node.end_position()))
            }
            // ES6 class式: constructorの位置を返す（存在すれば）
            "class" => {
                if let Some(constructor) = self.get_constructor_from_class(node, source) {
                    Some((constructor.start_position(), constructor.end_position()))
                } else {
                    Some((node.start_position(), node.end_position()))
                }
            }
            "array" => {
                // DI配列: ['$http', function($http) { ... }] または ['$http', class { constructor($http) {} }]
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "function_expression" || child.kind() == "arrow_function" {
                        return Some((child.start_position(), child.end_position()));
                    }
                    if child.kind() == "class" {
                        // class式の場合はconstructorの位置を返す
                        if let Some(constructor) = self.get_constructor_from_class(child, source) {
                            return Some((constructor.start_position(), constructor.end_position()));
                        } else {
                            return Some((child.start_position(), child.end_position()));
                        }
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
                // ファイル内の関数宣言または変数宣言またはclass宣言を探す
                self.find_function_declaration_position(root, source, &func_name)
            }
            _ => None,
        }
    }

    /// ファイル内で指定された名前の関数宣言、変数宣言（関数式）、またはclass宣言を探す
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
            // ES6 class宣言: constructorの位置を返す（存在すれば）
            "class_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    if self.node_text(name_node, source) == name {
                        // constructorがあればその位置、なければclass全体
                        if let Some(constructor) = self.get_constructor_from_class(node, source) {
                            return Some((constructor.start_position(), constructor.end_position()));
                        }
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

    /// AngularJS 1.5+ の `.component()` 呼び出しを解析する
    ///
    /// 認識パターン:
    /// ```javascript
    /// // パターン1: 文字列リテラル + オブジェクト
    /// .component('myComponent', { bindings: {}, controller: ..., templateUrl: '...' })
    ///
    /// // パターン2: 識別子.name + 識別子.config（ES6 module パターン）
    /// // UserDetails.js: export default { name: 'userDetails', config: {...} }
    /// // Users.js: .component(UserDetails.name, UserDetails.config)
    /// ```
    pub(super) fn extract_angular_component(&self, node: Node, source: &str, uri: &Url, ctx: &mut AnalyzerContext) {
        if let Some(args) = node.child_by_field_name("arguments") {
            if let Some(first_arg) = args.named_child(0) {
                // パターン1: .component('myComponent', {...})
                if first_arg.kind() == "string" {
                    let component_name = self.extract_string_value(first_arg, source);
                    self.register_component_symbol(
                        &component_name,
                        first_arg,
                        args.named_child(1),
                        source,
                        uri,
                        ctx,
                    );
                }
                // パターン2: .component(Identifier.name, Identifier.config)
                else if first_arg.kind() == "member_expression" {
                    if let Some(property) = first_arg.child_by_field_name("property") {
                        let prop_name = self.node_text(property, source);
                        if prop_name == "name" {
                            // Identifier.name パターン - export default オブジェクトから名前を解決
                            if let Some(object) = first_arg.child_by_field_name("object") {
                                let import_name = self.node_text(object, source);
                                // インデックスから export default オブジェクトの name プロパティを検索
                                if let Some(component_name) = self.index.get_exported_component_name(&import_name) {
                                    self.register_component_symbol(
                                        &component_name,
                                        first_arg,
                                        args.named_child(1),
                                        source,
                                        uri,
                                        ctx,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// コンポーネントシンボルを登録する
    fn register_component_symbol(
        &self,
        component_name: &str,
        name_node: Node,
        config_node: Option<Node>,
        source: &str,
        uri: &Url,
        _ctx: &mut AnalyzerContext,
    ) {
        let name_start = name_node.start_position();
        let name_end = name_node.end_position();

        // 定義位置はconfig objectがあればその位置、なければname_nodeの位置
        let (start, end) = if let Some(config) = config_node {
            // config オブジェクトから templateUrl, bindings を抽出
            if config.kind() == "object" {
                self.extract_component_template_url(config, source, uri, Some(component_name));
            }
            (config.start_position(), config.end_position())
        } else {
            (name_start, name_end)
        };

        let docs = self.extract_jsdoc_for_line(name_start.row, source);

        let symbol = Symbol {
            name: component_name.to_string(),
            kind: SymbolKind::Component,
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

    /// コンポーネント設定オブジェクトから templateUrl, controller, controllerAs, bindings を抽出
    fn extract_component_template_url(&self, config_node: Node, source: &str, uri: &Url, component_name: Option<&str>) {
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
                                        // 最後の要素が function/class の場合はNone（インラインコントローラー）
                                    }
                                }
                                // controller: function() {} または controller: class {} はNoneのまま
                                // （インラインコントローラーは別途this.methodパターンで処理）
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
            self.extract_component_bindings(bindings, source, uri, prefix);
        }
    }

    /// コンポーネントのbindingsオブジェクトからバインディングを抽出
    ///
    /// 認識パターン:
    /// ```javascript
    /// bindings: { users: '<', selected: '<', showDetails: '&onSelected' }
    /// ```
    fn extract_component_bindings(&self, bindings_node: Node, source: &str, uri: &Url, controller_name: &str) {
        let mut cursor = bindings_node.walk();
        for child in bindings_node.children(&mut cursor) {
            if child.kind() == "pair" {
                if let Some(key) = child.child_by_field_name("key") {
                    let key_text = self.node_text(key, source);
                    // 識別子の場合はそのまま、文字列の場合はクォートを除去
                    let binding_name = key_text.trim_matches(|c| c == '"' || c == '\'');

                    // バインディングタイプを取得（'<', '=', '@', '&'など）
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

                    // ControllerName.bindingName として登録
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
}
