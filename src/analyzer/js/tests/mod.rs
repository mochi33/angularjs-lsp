use std::sync::Arc;

use tower_lsp::lsp_types::Url;

use crate::index::Index;
use crate::model::SymbolKind;

use super::AngularJsAnalyzer;

fn test_uri() -> Url {
    Url::parse("file:///test.js").unwrap()
}

fn analyze(source: &str) -> Arc<Index> {
    let index = Arc::new(Index::new());
    let analyzer = AngularJsAnalyzer::new(Arc::clone(&index));
    analyzer.analyze_document(&test_uri(), source);
    index
}

/// ヘルパー: 指定名・指定種類の定義が存在するか
fn has_definition(index: &Index, name: &str, kind: SymbolKind) -> bool {
    index
        .definitions
        .get_definitions(name)
        .iter()
        .any(|s| s.kind == kind)
}

/// ヘルパー: 指定URIの全コントローラースコープを取得
fn get_controller_scopes(index: &Index) -> Vec<crate::model::ControllerScope> {
    index.controllers.get_all_controller_scopes()
}

/// ヘルパー: 指定名のコントローラースコープを取得
fn get_scope_for(index: &Index, name: &str) -> Option<crate::model::ControllerScope> {
    get_controller_scopes(index)
        .into_iter()
        .find(|s| s.name == name)
}

// ==========================================================================
// 基本パターン: DI配列記法
// ==========================================================================

#[test]
fn test_di_array_controller() {
    let index = analyze(
        r#"
angular.module('app', [])
.controller('MyCtrl', ['$scope', 'UserService', function($scope, UserService) {
    $scope.name = 'test';
}]);
"#,
    );

    assert!(has_definition(&index, "MyCtrl", SymbolKind::Controller));
    let scope = get_scope_for(&index, "MyCtrl").expect("controller scope should exist");
    assert!(scope.injected_services.contains(&"UserService".to_string()));
}

#[test]
fn test_di_array_service() {
    let index = analyze(
        r#"
angular.module('app', [])
.service('DataService', ['$http', 'AuthService', function($http, AuthService) {
    this.getData = function() {};
}]);
"#,
    );

    assert!(has_definition(&index, "DataService", SymbolKind::Service));
    let scope = get_scope_for(&index, "DataService").expect("service scope should exist");
    assert!(scope.injected_services.contains(&"AuthService".to_string()));
}

#[test]
fn test_di_array_factory() {
    let index = analyze(
        r#"
angular.module('app', [])
.factory('AuthFactory', ['$http', 'TokenService', function($http, TokenService) {
    return { login: function() {} };
}]);
"#,
    );

    assert!(has_definition(&index, "AuthFactory", SymbolKind::Factory));
    let scope = get_scope_for(&index, "AuthFactory").expect("factory scope should exist");
    assert!(scope.injected_services.contains(&"TokenService".to_string()));
}

// ==========================================================================
// 直接関数記法
// ==========================================================================

#[test]
fn test_direct_function_controller() {
    let index = analyze(
        r#"
angular.module('app', [])
.controller('DirectCtrl', function($scope, MyService) {
    $scope.value = 1;
});
"#,
    );

    assert!(has_definition(&index, "DirectCtrl", SymbolKind::Controller));
    let scope = get_scope_for(&index, "DirectCtrl").expect("controller scope should exist");
    assert!(scope.injected_services.contains(&"MyService".to_string()));
}

// ==========================================================================
// 関数参照パターン（identifier → 関数宣言）
// ==========================================================================

#[test]
fn test_function_ref_controller() {
    let index = analyze(
        r#"
function RefCtrl($scope, UserService) {
    $scope.users = [];
}

angular.module('app', [])
.controller('RefCtrl', RefCtrl);
"#,
    );

    assert!(has_definition(&index, "RefCtrl", SymbolKind::Controller));
    let scope = get_scope_for(&index, "RefCtrl").expect("controller scope should exist");
    assert!(scope.injected_services.contains(&"UserService".to_string()));
}

// ==========================================================================
// class参照パターン（identifier → class宣言）
// ==========================================================================

#[test]
fn test_class_ref_controller() {
    let index = analyze(
        r#"
class ClassCtrl {
    constructor($scope, DataService) {
        this.data = [];
    }
}

angular.module('app', [])
.controller('ClassCtrl', ClassCtrl);
"#,
    );

    assert!(has_definition(&index, "ClassCtrl", SymbolKind::Controller));
    let scope = get_scope_for(&index, "ClassCtrl").expect("controller scope should exist");
    assert!(scope.injected_services.contains(&"DataService".to_string()));
}

// ==========================================================================
// class式パターン（インライン class）
// ==========================================================================

#[test]
fn test_inline_class_controller() {
    let index = analyze(
        r#"
angular.module('app', [])
.controller('InlineClassCtrl', ['$scope', 'SomeService', class {
    constructor($scope, SomeService) {
        this.items = [];
    }
}]);
"#,
    );

    assert!(has_definition(
        &index,
        "InlineClassCtrl",
        SymbolKind::Controller
    ));
    let scope = get_scope_for(&index, "InlineClassCtrl").expect("controller scope should exist");
    assert!(scope.injected_services.contains(&"SomeService".to_string()));
}

// ==========================================================================
// 一時変数解決パターン（identifier → 変数 → 配列）
// これが今回の新機能: 一時変数の中身を見て判断する
// ==========================================================================

#[test]
fn test_variable_holding_di_array_for_run() {
    let index = analyze(
        r#"
var runDeps = ['$rootScope', function($rootScope) {
    $rootScope.appName = 'MyApp';
}];

angular.module('app', [])
.run(runDeps);
"#,
    );

    // run() には名前がないので定義は登録されないが、
    // コントローラースコープは作られない（runだから）。
    // DiScopeとしてcontextに追加されるが、indexのcontrollerには追加されない。
    // → モジュール定義が登録されていることを確認
    assert!(has_definition(&index, "app", SymbolKind::Module));
}

#[test]
fn test_variable_holding_function_for_controller() {
    let index = analyze(
        r#"
var MyCtrlFn = function($scope, OrderService) {
    $scope.orders = [];
};

angular.module('app', [])
.controller('VarCtrl', MyCtrlFn);
"#,
    );

    assert!(has_definition(&index, "VarCtrl", SymbolKind::Controller));
    let scope = get_scope_for(&index, "VarCtrl").expect("controller scope should exist");
    assert!(scope.injected_services.contains(&"OrderService".to_string()));
}

#[test]
fn test_variable_holding_function_for_service() {
    let index = analyze(
        r#"
var DataSvcImpl = function($http, CacheService) {
    this.fetch = function() {};
};

angular.module('app', [])
.service('DataSvc', DataSvcImpl);
"#,
    );

    assert!(has_definition(&index, "DataSvc", SymbolKind::Service));
    let scope = get_scope_for(&index, "DataSvc").expect("service scope should exist");
    assert!(scope.injected_services.contains(&"CacheService".to_string()));
}

// ==========================================================================
// $inject パターン
// ==========================================================================

#[test]
fn test_inject_pattern_controller() {
    let index = analyze(
        r#"
function InjectCtrl($scope, ApiService) {
    $scope.data = [];
}
InjectCtrl.$inject = ['$scope', 'ApiService'];

angular.module('app', [])
.controller('InjectCtrl', InjectCtrl);
"#,
    );

    assert!(has_definition(&index, "InjectCtrl", SymbolKind::Controller));
    let scope = get_scope_for(&index, "InjectCtrl").expect("controller scope should exist");
    assert!(scope.injected_services.contains(&"ApiService".to_string()));
}

// ==========================================================================
// .run() / .config() パターン
// ==========================================================================

#[test]
fn test_run_with_di_array() {
    let index = analyze(
        r#"
angular.module('app', [])
.run(['$rootScope', 'AuthService', function($rootScope, AuthService) {
    $rootScope.isLoggedIn = false;
}]);
"#,
    );

    // run はコントローラーではないのでcontroller_scopeには登録されない
    // ただし定義はモジュールとして存在する
    assert!(has_definition(&index, "app", SymbolKind::Module));
}

#[test]
fn test_run_with_function_ref() {
    let index = analyze(
        r#"
function AppInit($rootScope) {
    $rootScope.ready = true;
}

angular.module('app', [])
.run(AppInit);
"#,
    );

    assert!(has_definition(&index, "app", SymbolKind::Module));
}

// ==========================================================================
// その他の登録タイプ
// ==========================================================================

#[test]
fn test_directive_definition() {
    let index = analyze(
        r#"
angular.module('app', [])
.directive('myDirective', ['$compile', 'UtilService', function($compile, UtilService) {
    return { restrict: 'E' };
}]);
"#,
    );

    assert!(has_definition(&index, "myDirective", SymbolKind::Directive));
}

#[test]
fn test_filter_definition() {
    let index = analyze(
        r#"
angular.module('app', [])
.filter('myFilter', function() {
    return function(input) { return input; };
});
"#,
    );

    assert!(has_definition(&index, "myFilter", SymbolKind::Filter));
}

#[test]
fn test_constant_definition() {
    let index = analyze(
        r#"
angular.module('app', [])
.constant('API_URL', 'https://api.example.com');
"#,
    );

    assert!(has_definition(&index, "API_URL", SymbolKind::Constant));
}

#[test]
fn test_value_definition() {
    let index = analyze(
        r#"
angular.module('app', [])
.value('appConfig', { debug: true });
"#,
    );

    assert!(has_definition(&index, "appConfig", SymbolKind::Value));
}

#[test]
fn test_provider_definition() {
    let index = analyze(
        r#"
angular.module('app', [])
.provider('myProvider', function() {
    this.$get = function() { return {}; };
});
"#,
    );

    assert!(has_definition(&index, "myProvider", SymbolKind::Provider));
}

// ==========================================================================
// チェーン呼び出しパターン
// ==========================================================================

#[test]
fn test_chained_registration() {
    let index = analyze(
        r#"
angular.module('app', [])
.controller('Ctrl1', ['$scope', function($scope) {}])
.service('Svc1', ['$http', function($http) {}])
.factory('Fac1', function() { return {}; });
"#,
    );

    assert!(has_definition(&index, "Ctrl1", SymbolKind::Controller));
    assert!(has_definition(&index, "Svc1", SymbolKind::Service));
    assert!(has_definition(&index, "Fac1", SymbolKind::Factory));
}

// ==========================================================================
// $routeProvider.when() パターン
// ==========================================================================

#[test]
fn test_route_with_inline_controller() {
    let index = analyze(
        r#"
angular.module('app', [])
.config(['$routeProvider', function($routeProvider) {
    $routeProvider.when('/home', {
        templateUrl: 'home.html',
        controller: ['$scope', 'HomeService', function($scope, HomeService) {
            $scope.welcome = 'Hello';
        }]
    });
}]);
"#,
    );

    // route の inline controller は "route" という名前で登録される
    let scope = get_scope_for(&index, "route").expect("route controller scope should exist");
    assert!(scope.injected_services.contains(&"HomeService".to_string()));
}

#[test]
fn test_route_with_string_controller_ref() {
    let index = analyze(
        r#"
angular.module('app', [])
.config(['$routeProvider', function($routeProvider) {
    $routeProvider.when('/users', {
        templateUrl: 'users.html',
        controller: 'UsersCtrl'
    });
}]);
"#,
    );

    // controller: 'UsersCtrl' は参照として登録される
    let refs = index.definitions.get_references("UsersCtrl");
    assert!(!refs.is_empty(), "reference to UsersCtrl should be registered");
}

// ==========================================================================
// 一時変数解決: 複合ケース
// ==========================================================================

#[test]
fn test_variable_holding_di_array_for_controller() {
    let index = analyze(
        r#"
var ctrlDeps = ['$scope', 'ProductService', function($scope, ProductService) {
    $scope.products = [];
}];

angular.module('app', [])
.controller('VarArrayCtrl', ctrlDeps);
"#,
    );

    assert!(has_definition(
        &index,
        "VarArrayCtrl",
        SymbolKind::Controller
    ));
    let scope = get_scope_for(&index, "VarArrayCtrl").expect("controller scope should exist");
    assert!(scope
        .injected_services
        .contains(&"ProductService".to_string()));
}

#[test]
fn test_variable_holding_arrow_function() {
    let index = analyze(
        r#"
var arrowFn = ($scope, LogService) => {
    $scope.logs = [];
};

angular.module('app', [])
.controller('ArrowCtrl', arrowFn);
"#,
    );

    assert!(has_definition(&index, "ArrowCtrl", SymbolKind::Controller));
    let scope = get_scope_for(&index, "ArrowCtrl").expect("controller scope should exist");
    assert!(scope.injected_services.contains(&"LogService".to_string()));
}

#[test]
fn test_const_variable_holding_function() {
    let index = analyze(
        r#"
const svcImpl = function($http, CacheService) {
    this.get = function() {};
};

angular.module('app', [])
.service('ConstSvc', svcImpl);
"#,
    );

    assert!(has_definition(&index, "ConstSvc", SymbolKind::Service));
    let scope = get_scope_for(&index, "ConstSvc").expect("service scope should exist");
    assert!(scope.injected_services.contains(&"CacheService".to_string()));
}

#[test]
fn test_let_variable_holding_function() {
    let index = analyze(
        r#"
let factoryFn = function($q, DataService) {
    return { process: function() {} };
};

angular.module('app', [])
.factory('LetFactory', factoryFn);
"#,
    );

    assert!(has_definition(&index, "LetFactory", SymbolKind::Factory));
    let scope = get_scope_for(&index, "LetFactory").expect("factory scope should exist");
    assert!(scope.injected_services.contains(&"DataService".to_string()));
}

// ==========================================================================
// $scope ネストされたプロパティ代入
// ==========================================================================

#[test]
fn test_nested_scope_property_assignment() {
    let index = analyze(
        r#"
angular.module('app', []).controller('NestedCtrl', ['$scope', function($scope) {
    $scope.user = {};
    $scope.user.name = 'test';
    $scope.user.email = 'test@example.com';
    $scope.items = [];
}]);
"#,
    );

    // 第1レベルのプロパティは定義される
    assert!(has_definition(
        &index,
        "NestedCtrl.$scope.user",
        SymbolKind::ScopeProperty
    ));
    assert!(has_definition(
        &index,
        "NestedCtrl.$scope.items",
        SymbolKind::ScopeProperty
    ));

    // ネストされたプロパティも定義される
    assert!(has_definition(
        &index,
        "NestedCtrl.$scope.user.name",
        SymbolKind::ScopeProperty
    ));
    assert!(has_definition(
        &index,
        "NestedCtrl.$scope.user.email",
        SymbolKind::ScopeProperty
    ));
}

#[test]
fn test_nested_scope_method_assignment() {
    let index = analyze(
        r#"
angular.module('app', []).controller('NestedMethodCtrl', ['$scope', function($scope) {
    $scope.actions = {};
    $scope.actions.save = function(data) {};
    $scope.actions.load = function() {};
}]);
"#,
    );

    assert!(has_definition(
        &index,
        "NestedMethodCtrl.$scope.actions",
        SymbolKind::ScopeProperty
    ));
    assert!(has_definition(
        &index,
        "NestedMethodCtrl.$scope.actions.save",
        SymbolKind::ScopeMethod
    ));
    assert!(has_definition(
        &index,
        "NestedMethodCtrl.$scope.actions.load",
        SymbolKind::ScopeMethod
    ));
}

#[test]
fn test_deeply_nested_scope_property() {
    let index = analyze(
        r#"
angular.module('app', []).controller('DeepCtrl', ['$scope', function($scope) {
    $scope.a = {};
    $scope.a.b = {};
    $scope.a.b.c = 'deep';
}]);
"#,
    );

    assert!(has_definition(
        &index,
        "DeepCtrl.$scope.a",
        SymbolKind::ScopeProperty
    ));
    assert!(has_definition(
        &index,
        "DeepCtrl.$scope.a.b",
        SymbolKind::ScopeProperty
    ));
    assert!(has_definition(
        &index,
        "DeepCtrl.$scope.a.b.c",
        SymbolKind::ScopeProperty
    ));
}

// ==========================================================================
// DI参照（$以外のサービス名）が正しく参照登録されるか
// ==========================================================================

#[test]
fn test_di_array_registers_references() {
    let index = analyze(
        r#"
angular.module('app', [])
.controller('RefTest', ['$scope', 'MyService', 'OtherService', function($scope, MyService, OtherService) {
}]);
"#,
    );

    let my_refs = index.definitions.get_references("MyService");
    assert!(
        !my_refs.is_empty(),
        "MyService should have a reference registered"
    );

    let other_refs = index.definitions.get_references("OtherService");
    assert!(
        !other_refs.is_empty(),
        "OtherService should have a reference registered"
    );
}

// ==========================================================================
// コンポーネントの定義
// ==========================================================================

#[test]
fn test_component_definition() {
    let index = analyze(
        r#"
angular.module('app', [])
.component('userList', {
    templateUrl: 'user-list.html',
    controller: 'UserListCtrl',
    bindings: {
        users: '<',
        onSelect: '&'
    }
});
"#,
    );

    assert!(has_definition(&index, "userList", SymbolKind::Component));
}

// ==========================================================================
// 複合テスト: sample.js フィクスチャ相当
// ==========================================================================

#[test]
fn test_full_module_registration() {
    let index = analyze(
        r#"
angular.module('myApp', ['ngRoute', 'myApp.services'])

.controller('MainCtrl', ['$scope', 'UserService', function($scope, UserService) {
    $scope.users = [];
}])

.service('UserService', ['$http', '$q', function($http, $q) {
    this.getAll = function() {};
}])

.directive('userCard', ['UserService', function(UserService) {
    return { restrict: 'E' };
}])

.factory('AuthService', ['$http', '$q', function($http, $q) {
    return { login: function() {} };
}])

.constant('API_URL', 'https://api.example.com')

.value('appConfig', { debug: true });
"#,
    );

    assert!(has_definition(&index, "myApp", SymbolKind::Module));
    assert!(has_definition(&index, "MainCtrl", SymbolKind::Controller));
    assert!(has_definition(&index, "UserService", SymbolKind::Service));
    assert!(has_definition(&index, "userCard", SymbolKind::Directive));
    assert!(has_definition(&index, "AuthService", SymbolKind::Factory));
    assert!(has_definition(&index, "API_URL", SymbolKind::Constant));
    assert!(has_definition(&index, "appConfig", SymbolKind::Value));

    // DI参照チェック
    let user_refs = index.definitions.get_references("UserService");
    assert!(
        !user_refs.is_empty(),
        "UserService should be referenced from controllers"
    );
}
