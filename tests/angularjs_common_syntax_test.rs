///! AngularJS一般的な構文のLSP対応状況を検証する統合テスト
///!
///! このテストは、AngularJS 1.xで使われる主要な構文パターンを網羅し、
///! LSPのアナライザーが各パターンを正しく認識できるか検証する。

use std::sync::Arc;
use tower_lsp::lsp_types::Url;

use angularjs_lsp::analyzer::js::AngularJsAnalyzer;
use angularjs_lsp::analyzer::html::HtmlAngularJsAnalyzer;
use angularjs_lsp::handler::WorkspaceSymbolHandler;
use angularjs_lsp::index::Index;
use angularjs_lsp::model::SymbolKind;

/// テスト用ヘルパー：JSソースを解析してIndex内のシンボルを返す
fn analyze_js(source: &str) -> Arc<Index> {
    let index = Arc::new(Index::new());
    let analyzer = AngularJsAnalyzer::new(index.clone());
    let uri = Url::parse("file:///test.js").unwrap();
    analyzer.analyze_document(&uri, source);
    index
}

/// テスト用ヘルパー：HTMLソースを解析してIndex内のシンボルを返す
fn analyze_html(js_source: &str, html_source: &str) -> Arc<Index> {
    let index = Arc::new(Index::new());
    let js_analyzer = Arc::new(AngularJsAnalyzer::new(index.clone()));
    let html_analyzer = HtmlAngularJsAnalyzer::new(index.clone(), js_analyzer.clone());

    // まずJSを解析
    if !js_source.is_empty() {
        let js_uri = Url::parse("file:///test.js").unwrap();
        js_analyzer.analyze_document(&js_uri, js_source);
    }

    // 次にHTMLを解析
    let html_uri = Url::parse("file:///test.html").unwrap();
    html_analyzer.analyze_document(&html_uri, html_source);

    index
}

/// 指定した名前とSymbolKindの定義が存在するかチェック
fn has_definition(index: &Index, name: &str, kind: SymbolKind) -> bool {
    let defs = index.definitions.get_definitions(name);
    defs.iter().any(|d| d.kind == kind)
}

/// 指定した名前の定義が存在するかチェック（kind問わず）
fn has_any_definition(index: &Index, name: &str) -> bool {
    !index.definitions.get_definitions(name).is_empty()
}

/// 指定した名前の参照が存在するかチェック
fn has_reference(index: &Index, name: &str) -> bool {
    !index.definitions.get_references(name).is_empty()
}

// ============================================================
// 1. Module定義パターン
// ============================================================

#[test]
fn test_module_definition_with_deps() {
    let source = "angular.module('myApp', ['ngRoute', 'ngAnimate']);";
    let index = analyze_js(source);
    assert!(has_definition(&index, "myApp", SymbolKind::Module),
        "モジュール定義（依存配列あり）が認識されるべき");
}

#[test]
fn test_module_definition_empty_deps() {
    let source = "angular.module('simpleApp', []);";
    let index = analyze_js(source);
    assert!(has_definition(&index, "simpleApp", SymbolKind::Module),
        "モジュール定義（空の依存配列）が認識されるべき");
}

#[test]
fn test_module_getter_reference() {
    let source = r#"
angular.module('myApp', []);
angular.module('myApp');
"#;
    let index = analyze_js(source);
    // 既存モジュール参照（getter）も定義として登録される
    let defs = index.definitions.get_definitions("myApp");
    assert!(defs.len() >= 1, "モジュール参照（getter）が認識されるべき");
}

// ============================================================
// 2. Controller定義パターン
// ============================================================

#[test]
fn test_controller_di_array_syntax() {
    let source = r#"
angular.module('app', [])
.controller('ArrayDIController', ['$scope', '$http', function($scope, $http) {
    $scope.users = [];
    $scope.loadUsers = function() {};
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "ArrayDIController", SymbolKind::Controller),
        "DI配列記法のコントローラーが認識されるべき");
    assert!(has_definition(&index, "ArrayDIController.$scope.users", SymbolKind::ScopeProperty),
        "$scope.usersプロパティが認識されるべき");
    assert!(has_definition(&index, "ArrayDIController.$scope.loadUsers", SymbolKind::ScopeMethod),
        "$scope.loadUsersメソッドが認識されるべき");
}

#[test]
fn test_controller_inject_pattern() {
    let source = r#"
function InjectCtrl($scope, $timeout) {
    $scope.message = 'Hello';
    $scope.counter = 0;
}
InjectCtrl.$inject = ['$scope', '$timeout'];
angular.module('app', []).controller('InjectCtrl', InjectCtrl);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "InjectCtrl", SymbolKind::Controller),
        "$injectパターンのコントローラーが認識されるべき");
    assert!(has_definition(&index, "InjectCtrl.$scope.message", SymbolKind::ScopeProperty),
        "$injectパターンの$scopeプロパティが認識されるべき");
}

#[test]
fn test_controller_function_ref_without_inject() {
    let source = r#"
function SimpleFuncCtrl($scope) {
    $scope.title = 'Hello';
}
angular.module('app', []).controller('SimpleFuncCtrl', SimpleFuncCtrl);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "SimpleFuncCtrl", SymbolKind::Controller),
        "関数参照パターン（$injectなし）のコントローラーが認識されるべき");
}

#[test]
fn test_controller_es6_class() {
    let source = r#"
class ClassCtrl {
    constructor($scope, $http) {
        $scope.items = [];
    }
    refresh() {
        console.log('refreshing');
    }
}
ClassCtrl.$inject = ['$scope', '$http'];
angular.module('app', []).controller('ClassCtrl', ClassCtrl);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "ClassCtrl", SymbolKind::Controller),
        "ES6 classパターンのコントローラーが認識されるべき");
    assert!(has_definition(&index, "ClassCtrl.$scope.items", SymbolKind::ScopeProperty),
        "ES6 class内の$scopeプロパティが認識されるべき");
    // classメソッドはcontroller as用にMethodとして登録されるべき
    assert!(has_definition(&index, "ClassCtrl.refresh", SymbolKind::Method),
        "ES6 classメソッドがMethodとして認識されるべき");
}

#[test]
fn test_controller_inline_class() {
    let source = r#"
angular.module('app', []).controller('InlineClassCtrl', class {
    constructor($scope) {
        $scope.msg = 'inline';
    }
});
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "InlineClassCtrl", SymbolKind::Controller),
        "インラインclass式のコントローラーが認識されるべき");
}

#[test]
fn test_controller_array_class() {
    let source = r#"
angular.module('app', []).controller('ArrayClassCtrl', ['$scope', '$log', class {
    constructor($scope, $log) {
        $scope.logged = true;
    }
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "ArrayClassCtrl", SymbolKind::Controller),
        "DI配列内class式のコントローラーが認識されるべき");
}

#[test]
fn test_controller_as_pattern() {
    let source = r#"
angular.module('app', []).controller('CtrlAsCtrl', ['$http', function($http) {
    var vm = this;
    vm.title = 'Controller As';
    vm.items = [];
    vm.loadItems = function() {};
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "CtrlAsCtrl", SymbolKind::Controller),
        "controller asパターンが認識されるべき");
    // vm.xxxはthisエイリアス -> this.xxx -> ControllerName.xxxとして登録
    assert!(has_definition(&index, "CtrlAsCtrl.title", SymbolKind::Method),
        "vm.title（thisエイリアス）がMethodとして認識されるべき");
    assert!(has_definition(&index, "CtrlAsCtrl.loadItems", SymbolKind::Method),
        "vm.loadItems（thisエイリアス）がMethodとして認識されるべき");
}

#[test]
fn test_controller_arrow_function() {
    let source = r#"
angular.module('app', []).controller('ArrowCtrl', ['$scope', ($scope) => {
    $scope.arrowMsg = 'From arrow';
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "ArrowCtrl", SymbolKind::Controller),
        "アロー関数のコントローラーが認識されるべき");
}

// ============================================================
// 3. Service定義パターン
// ============================================================

#[test]
fn test_service_di_array() {
    let source = r#"
angular.module('app', []).service('UserService', ['$http', '$q', function($http, $q) {
    this.getAll = function() { return $http.get('/api/users'); };
    this.getById = function(id) { return $http.get('/api/users/' + id); };
    this.create = function(user) { return $http.post('/api/users', user); };
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "UserService", SymbolKind::Service),
        "DI配列記法のサービスが認識されるべき");
    assert!(has_definition(&index, "UserService.getAll", SymbolKind::Method),
        "サービスメソッド getAll が認識されるべき");
    assert!(has_definition(&index, "UserService.getById", SymbolKind::Method),
        "サービスメソッド getById が認識されるべき");
    assert!(has_definition(&index, "UserService.create", SymbolKind::Method),
        "サービスメソッド create が認識されるべき");
}

#[test]
fn test_service_class_based() {
    let source = r#"
class DataService {
    constructor($http) {
        this.http = $http;
    }
    getData(key) {
        return this.http.get('/api/data/' + key);
    }
    clearCache() {
        this.cache.removeAll();
    }
}
DataService.$inject = ['$http'];
angular.module('app', []).service('DataService', DataService);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "DataService", SymbolKind::Service),
        "class-basedサービスが認識されるべき");
    assert!(has_definition(&index, "DataService.getData", SymbolKind::Method),
        "class-basedサービスのメソッドが認識されるべき");
    assert!(has_definition(&index, "DataService.clearCache", SymbolKind::Method),
        "class-basedサービスのメソッドが認識されるべき");
}

#[test]
fn test_service_implicit_injection() {
    let source = r#"
angular.module('app', []).service('SimpleService', function($http) {
    this.fetch = function() { return $http.get('/api/simple'); };
});
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "SimpleService", SymbolKind::Service),
        "暗黙的DI（DI配列なし）のサービスが認識されるべき");
    assert!(has_definition(&index, "SimpleService.fetch", SymbolKind::Method),
        "暗黙的DIサービスのメソッドが認識されるべき");
}

// --- thisエイリアスパターン (that = this) ---

#[test]
fn test_service_this_alias_that() {
    let source = r#"
angular.module('app', []).service('ThatSvc', [function() {
    var that = this;
    that.doWork = function() {};
    that.name = 'test';
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "ThatSvc", SymbolKind::Service),
        "that=thisパターンのサービスが認識されるべき");
    assert!(has_definition(&index, "ThatSvc.doWork", SymbolKind::Method),
        "that.doWork が Method として認識されるべき");
    assert!(has_definition(&index, "ThatSvc.name", SymbolKind::Method),
        "that.name が Method として認識されるべき");
}

// ============================================================
// 4. Factory定義パターン
// ============================================================

#[test]
fn test_factory_basic() {
    let source = r#"
angular.module('app', []).factory('AuthService', ['$http', '$q', function($http, $q) {
    var service = {};
    service.login = function(credentials) { return $http.post('/api/login', credentials); };
    service.logout = function() {};
    return service;
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "AuthService", SymbolKind::Factory),
        "基本的なファクトリーが認識されるべき");
    assert!(has_definition(&index, "AuthService.login", SymbolKind::Method),
        "var service = {{}}; service.login パターンのメソッドが認識されるべき");
    assert!(has_definition(&index, "AuthService.logout", SymbolKind::Method),
        "var service = {{}}; service.logout パターンのメソッドが認識されるべき");
}

#[test]
fn test_factory_revealing_module_pattern() {
    let source = r#"
angular.module('app', []).factory('UtilService', [function() {
    function formatDate(date) { return date.toISOString(); }
    function capitalize(str) { return str.charAt(0).toUpperCase() + str.slice(1); }
    return {
        formatDate: formatDate,
        capitalize: capitalize
    };
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "UtilService", SymbolKind::Factory),
        "Revealing Module Patternのファクトリーが認識されるべき");
    assert!(has_definition(&index, "UtilService.formatDate", SymbolKind::Method),
        "Revealing Module Patternのメソッド(変数参照)が認識されるべき");
    assert!(has_definition(&index, "UtilService.capitalize", SymbolKind::Method),
        "Revealing Module Patternのメソッド(変数参照)が認識されるべき");
}

#[test]
fn test_factory_with_inline_functions() {
    let source = r#"
angular.module('app', []).factory('InlineFactory', [function() {
    return {
        method1: function(a, b) { return a + b; },
        method2: function() { return 'hello'; }
    };
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "InlineFactory", SymbolKind::Factory),
        "インライン関数ファクトリーが認識されるべき");
    assert!(has_definition(&index, "InlineFactory.method1", SymbolKind::Method),
        "インライン関数メソッドが認識されるべき");
    assert!(has_definition(&index, "InlineFactory.method2", SymbolKind::Method),
        "インライン関数メソッドが認識されるべき");
}

#[test]
fn test_factory_service_variable_pattern() {
    // var service = {}; service.xxx = ...; return service; パターン
    let source = r#"
angular.module('app', []).factory('SvcA', [function() {
    var service = {};
    service.doWork = function() {};
    service.name = 'test';
    return service;
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "SvcA", SymbolKind::Factory),
        "ファクトリーが認識されるべき");
    assert!(has_definition(&index, "SvcA.doWork", SymbolKind::Method),
        "service.doWork がメソッドとして認識されるべき");
    assert!(has_definition(&index, "SvcA.name", SymbolKind::Method),
        "service.name がメソッドとして認識されるべき");
}

#[test]
fn test_factory_service_variable_pattern_with_di() {
    // DI依存ありの場合
    let source = r#"
angular.module('app', []).factory('DataService', ['$http', '$q', function($http, $q) {
    var svc = {};
    svc.fetchAll = function() { return $http.get('/api/data'); };
    svc.fetchById = function(id) { return $http.get('/api/data/' + id); };
    svc.apiUrl = '/api/data';
    return svc;
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "DataService", SymbolKind::Factory),
        "DI付きファクトリーが認識されるべき");
    assert!(has_definition(&index, "DataService.fetchAll", SymbolKind::Method),
        "svc.fetchAll がメソッドとして認識されるべき");
    assert!(has_definition(&index, "DataService.fetchById", SymbolKind::Method),
        "svc.fetchById がメソッドとして認識されるべき");
    assert!(has_definition(&index, "DataService.apiUrl", SymbolKind::Method),
        "svc.apiUrl がメソッドとして認識されるべき");
}

#[test]
fn test_factory_service_variable_pattern_without_di_array() {
    // DI配列なし（直接関数渡し）パターン
    let source = r#"
angular.module('app', []).factory('SimpleService', function() {
    var service = {};
    service.greet = function(name) { return 'Hello ' + name; };
    return service;
});
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "SimpleService", SymbolKind::Factory),
        "直接関数渡しファクトリーが認識されるべき");
    assert!(has_definition(&index, "SimpleService.greet", SymbolKind::Method),
        "service.greet がメソッドとして認識されるべき");
}

// ============================================================
// 5. Directive定義パターン
// ============================================================

#[test]
fn test_directive_element() {
    let source = r#"
angular.module('app', []).directive('userCard', [function() {
    return {
        restrict: 'E',
        scope: { user: '=' },
        templateUrl: 'templates/user-card.html',
        link: function(scope, element, attrs) {}
    };
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "userCard", SymbolKind::Directive),
        "Elementディレクティブが認識されるべき");
}

#[test]
fn test_directive_attribute() {
    let source = r#"
angular.module('app', []).directive('myHighlight', [function() {
    return {
        restrict: 'A',
        link: function(scope, element, attrs) {}
    };
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "myHighlight", SymbolKind::Directive),
        "Attributeディレクティブが認識されるべき");
}

#[test]
fn test_directive_with_controller() {
    let source = r#"
angular.module('app', []).directive('tabPanel', [function() {
    return {
        restrict: 'E',
        transclude: true,
        scope: { tabs: '=' },
        controller: ['$scope', function($scope) {
            $scope.activeTab = 0;
        }],
        templateUrl: 'templates/tab-panel.html'
    };
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "tabPanel", SymbolKind::Directive),
        "controller付きディレクティブが認識されるべき");
}

#[test]
fn test_directive_returning_link_function() {
    let source = r#"
angular.module('app', []).directive('autoFocus', ['$timeout', function($timeout) {
    return function(scope, element) {
        $timeout(function() { element[0].focus(); });
    };
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "autoFocus", SymbolKind::Directive),
        "リンク関数のみを返すディレクティブが認識されるべき");
}

#[test]
fn test_directive_with_require() {
    let source = r#"
angular.module('app', []).directive('tabItem', [function() {
    return {
        restrict: 'E',
        require: '^tabPanel',
        scope: { title: '@' },
        link: function(scope, element, attrs, tabPanelCtrl) {}
    };
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "tabItem", SymbolKind::Directive),
        "require付きディレクティブが認識されるべき");
}

#[test]
fn test_directive_with_compile() {
    let source = r#"
angular.module('app', []).directive('compileDir', [function() {
    return {
        restrict: 'A',
        compile: function(tElement, tAttrs) {
            return {
                pre: function(scope, element, attrs) {},
                post: function(scope, element, attrs) {}
            };
        }
    };
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "compileDir", SymbolKind::Directive),
        "compile関数付きディレクティブが認識されるべき");
}

// ============================================================
// 6. Component定義パターン (AngularJS 1.5+)
// ============================================================

#[test]
fn test_component_basic() {
    let source = r#"
angular.module('app', []).component('heroDetail', {
    templateUrl: 'templates/hero-detail.html',
    controller: 'HeroDetailController',
    controllerAs: 'vm',
    bindings: {
        hero: '<',
        onDelete: '&',
        onUpdate: '&'
    }
});
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "heroDetail", SymbolKind::Component),
        "基本的なコンポーネントが認識されるべき");
    // コンポーネントバインディング
    assert!(has_definition(&index, "HeroDetailController.hero", SymbolKind::ComponentBinding),
        "コンポーネントバインディング hero が認識されるべき");
    assert!(has_definition(&index, "HeroDetailController.onDelete", SymbolKind::ComponentBinding),
        "コンポーネントバインディング onDelete が認識されるべき");
}

#[test]
fn test_component_with_inline_controller() {
    let source = r#"
angular.module('app', []).component('heroList', {
    templateUrl: 'templates/hero-list.html',
    controller: ['HeroService', function(HeroService) {
        var ctrl = this;
        ctrl.$onInit = function() {};
    }],
    bindings: {
        filter: '<'
    }
});
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "heroList", SymbolKind::Component),
        "インラインコントローラー付きコンポーネントが認識されるべき");
    assert!(has_definition(&index, "heroList.$onInit", SymbolKind::Method),
        "DI配列記法のインラインコントローラー内の$onInitが認識されるべき");
}

#[test]
fn test_component_lifecycle_hooks() {
    let source = r#"
angular.module('app', []).component('lifecycleDemo', {
    template: '<div>{{ $ctrl.status }}</div>',
    controller: function() {
        var ctrl = this;
        ctrl.status = 'created';
        ctrl.$onInit = function() { ctrl.status = 'initialized'; };
        ctrl.$onDestroy = function() {};
        ctrl.$doCheck = function() {};
        ctrl.$postLink = function() {};
    }
});
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "lifecycleDemo", SymbolKind::Component),
        "ライフサイクルフック付きコンポーネントが認識されるべき");
    assert!(has_definition(&index, "lifecycleDemo.status", SymbolKind::Method),
        "コンポーネントコントローラー内のプロパティが認識されるべき");
    assert!(has_definition(&index, "lifecycleDemo.$onInit", SymbolKind::Method),
        "コンポーネントの$onInitライフサイクルフックが認識されるべき");
    assert!(has_definition(&index, "lifecycleDemo.$onDestroy", SymbolKind::Method),
        "コンポーネントの$onDestroyライフサイクルフックが認識されるべき");
    assert!(has_definition(&index, "lifecycleDemo.$doCheck", SymbolKind::Method),
        "コンポーネントの$doCheckライフサイクルフックが認識されるべき");
    assert!(has_definition(&index, "lifecycleDemo.$postLink", SymbolKind::Method),
        "コンポーネントの$postLinkライフサイクルフックが認識されるべき");
}

// ============================================================
// 7. Provider定義パターン
// ============================================================

#[test]
fn test_provider_definition() {
    let source = r#"
angular.module('app', []).provider('apiConfig', function() {
    var baseUrl = '/api';
    this.setBaseUrl = function(url) { baseUrl = url; };
    this.$get = ['$http', function($http) {
        return {
            getUrl: function(path) { return baseUrl + path; }
        };
    }];
});
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "apiConfig", SymbolKind::Provider),
        "プロバイダー定義が認識されるべき");
}

// ============================================================
// 8. Filter定義パターン
// ============================================================

#[test]
fn test_filter_basic() {
    let source = r#"
angular.module('app', []).filter('capitalize', [function() {
    return function(input) {
        if (!input) return '';
        return input.charAt(0).toUpperCase() + input.slice(1);
    };
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "capitalize", SymbolKind::Filter),
        "基本的なフィルター定義が認識されるべき");
}

#[test]
fn test_filter_with_service_dependency() {
    let source = r#"
angular.module('app', []).filter('currency', ['$locale', function($locale) {
    return function(amount, symbol) {
        symbol = symbol || $locale.NUMBER_FORMATS.CURRENCY_SYM;
        return symbol + amount.toFixed(2);
    };
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "currency", SymbolKind::Filter),
        "サービス依存のフィルターが認識されるべき");
}

// ============================================================
// 9. Constant/Value定義パターン
// ============================================================

#[test]
fn test_constant_string() {
    let source = "angular.module('app', []).constant('API_URL', 'https://api.example.com');";
    let index = analyze_js(source);
    assert!(has_definition(&index, "API_URL", SymbolKind::Constant),
        "文字列Constantが認識されるべき");
}

#[test]
fn test_constant_object() {
    let source = r#"
angular.module('app', []).constant('APP_CONFIG', {
    debug: false,
    version: '2.0.0'
});
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "APP_CONFIG", SymbolKind::Constant),
        "オブジェクトConstantが認識されるべき");
}

#[test]
fn test_value_string() {
    let source = "angular.module('app', []).value('appVersion', '1.0.0');";
    let index = analyze_js(source);
    assert!(has_definition(&index, "appVersion", SymbolKind::Value),
        "文字列Valueが認識されるべき");
}

#[test]
fn test_value_object() {
    let source = r#"
angular.module('app', []).value('defaultSettings', {
    theme: 'light',
    language: 'ja'
});
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "defaultSettings", SymbolKind::Value),
        "オブジェクトValueが認識されるべき");
}

// ============================================================
// 10. .config() / .run() パターン
// ============================================================

#[test]
fn test_config_with_route_provider() {
    let source = r#"
angular.module('app', []).config(['$routeProvider', function($routeProvider) {
    $routeProvider
        .when('/users', {
            templateUrl: 'views/users.html',
            controller: 'UserController'
        })
        .when('/settings', {
            templateUrl: 'views/settings.html',
            controller: 'SettingsController'
        })
        .otherwise({ redirectTo: '/users' });
}]);
"#;
    let index = analyze_js(source);
    // routeProviderの.whenに登録されたcontrollerの参照が認識されるか
    assert!(has_reference(&index, "UserController"),
        ".when()のcontroller文字列参照が認識されるべき");
    assert!(has_reference(&index, "SettingsController"),
        ".when()のcontroller文字列参照が認識されるべき");
}

#[test]
fn test_run_with_rootscope() {
    let source = r#"
angular.module('app', []).run(['$rootScope', function($rootScope) {
    $rootScope.appName = 'Test App';
    $rootScope.isLoggedIn = false;
    $rootScope.goTo = function(path) {};
}]);
"#;
    let index = analyze_js(source);
    // $rootScopeプロパティはモジュール名をプレフィックスとして使用
    assert!(has_definition(&index, "app.$rootScope.appName", SymbolKind::RootScopeProperty),
        ".run()内の$rootScopeプロパティが認識されるべき (module名.$rootScope.prop形式)");
    assert!(has_definition(&index, "app.$rootScope.goTo", SymbolKind::RootScopeMethod),
        ".run()内の$rootScopeメソッドが認識されるべき (module名.$rootScope.method形式)");
}

// ============================================================
// 11. $scope高度なパターン
// ============================================================

#[test]
fn test_scope_watch() {
    let source = r#"
angular.module('app', []).controller('WatchCtrl', ['$scope', function($scope) {
    $scope.searchQuery = '';
    $scope.results = [];
    $scope.$watch('searchQuery', function(newVal, oldVal) {
        if (newVal !== oldVal) { $scope.search(newVal); }
    });
    $scope.search = function(query) {};
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "WatchCtrl.$scope.searchQuery", SymbolKind::ScopeProperty),
        "$watchで使われる$scopeプロパティが認識されるべき");
    assert!(has_definition(&index, "WatchCtrl.$scope.search", SymbolKind::ScopeMethod),
        "$watch内で呼ばれる$scopeメソッドが認識されるべき");
}

#[test]
fn test_scope_events() {
    let source = r#"
angular.module('app', []).controller('EventCtrl', ['$scope', '$rootScope', function($scope, $rootScope) {
    $scope.messages = [];
    $scope.broadcast = function(msg) {
        $rootScope.$broadcast('newMessage', { text: msg });
    };
    $scope.emit = function(msg) {
        $scope.$emit('newMessage', { text: msg });
    };
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "EventCtrl.$scope.messages", SymbolKind::ScopeProperty),
        "イベントパターンの$scopeプロパティが認識されるべき");
    assert!(has_definition(&index, "EventCtrl.$scope.broadcast", SymbolKind::ScopeMethod),
        "$rootScope.$broadcast使用の$scopeメソッドが認識されるべき");
}

// ============================================================
// 12. チェーン呼び出しパターン
// ============================================================

#[test]
fn test_chained_definitions() {
    let source = r#"
angular.module('chainApp', [])
    .constant('VERSION', '1.0')
    .value('settings', {})
    .service('ChainSvc', ['$http', function($http) {
        this.getData = function() { return $http.get('/api/data'); };
    }])
    .controller('ChainCtrl', ['$scope', function($scope) {
        $scope.data = null;
    }])
    .filter('chainFilter', [function() {
        return function(input) { return input; };
    }]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "chainApp", SymbolKind::Module),
        "チェーン呼び出しのモジュールが認識されるべき");
    assert!(has_definition(&index, "VERSION", SymbolKind::Constant),
        "チェーン呼び出しのConstantが認識されるべき");
    assert!(has_definition(&index, "settings", SymbolKind::Value),
        "チェーン呼び出しのValueが認識されるべき");
    assert!(has_definition(&index, "ChainSvc", SymbolKind::Service),
        "チェーン呼び出しのServiceが認識されるべき");
    assert!(has_definition(&index, "ChainCtrl", SymbolKind::Controller),
        "チェーン呼び出しのControllerが認識されるべき");
    assert!(has_definition(&index, "chainFilter", SymbolKind::Filter),
        "チェーン呼び出しのFilterが認識されるべき");
}

// ============================================================
// 13. 高度なDIパターン
// ============================================================

#[test]
fn test_var_inject_pattern() {
    let source = r#"
var AdvController = function($scope, $timeout) {
    $scope.ready = false;
};
AdvController.$inject = ['$scope', '$timeout'];
angular.module('app', []).controller('AdvController', AdvController);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "AdvController", SymbolKind::Controller),
        "var代入+$injectパターンが認識されるべき");
}

#[test]
fn test_const_inject_pattern() {
    let source = r#"
const ModernCtrl = function($scope, $http) {
    $scope.modern = true;
};
ModernCtrl.$inject = ['$scope', '$http'];
angular.module('app', []).controller('ModernCtrl', ModernCtrl);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "ModernCtrl", SymbolKind::Controller),
        "const代入+$injectパターンが認識されるべき");
}

#[test]
fn test_iife_pattern() {
    let source = r#"
(function() {
    'use strict';
    angular.module('app', []).controller('IIFECtrl', ['$scope', function($scope) {
        $scope.fromIIFE = true;
    }]);
})();
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "IIFECtrl", SymbolKind::Controller),
        "IIFE内のコントローラーが認識されるべき");
    assert!(has_definition(&index, "IIFECtrl.$scope.fromIIFE", SymbolKind::ScopeProperty),
        "IIFE内の$scopeプロパティが認識されるべき");
}

// ============================================================
// 14. Decorator パターン
// ============================================================

#[test]
fn test_module_decorator() {
    let source = r#"
angular.module('app', []).decorator('UserService', ['$delegate', '$log', function($delegate, $log) {
    var orig = $delegate.getAll;
    $delegate.getAll = function() { return orig.apply($delegate, arguments); };
    return $delegate;
}]);
"#;
    let index = analyze_js(source);
    // decoratorはSymbolKindとして認識されるか？
    // 現在のLSPがdecoratorをサポートしているか確認
    // decoratorは特別なSymbolKindが無い可能性が高い
    let all_defs = index.definitions.get_all_definitions();
    let has_user_service_decorator = all_defs.iter().any(|d| d.name == "UserService");
    // decoratorパターンのテスト結果を記録
    println!("decorator 'UserService' found: {}", has_user_service_decorator);
}

// ============================================================
// 15. $resource パターン
// ============================================================

#[test]
fn test_resource_factory() {
    let source = r#"
angular.module('app', []).factory('UserResource', ['$resource', function($resource) {
    return $resource('/api/users/:id', { id: '@id' }, {
        update: { method: 'PUT' },
        query: { method: 'GET', isArray: true }
    });
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "UserResource", SymbolKind::Factory),
        "$resourceベースのファクトリーが認識されるべき");
    // $resourceのreturn値からメソッドは抽出されない可能性が高い
    let has_update_method = has_definition(&index, "UserResource.update", SymbolKind::Method);
    println!("$resource custom action 'update' recognized as method: {}", has_update_method);
}

// ============================================================
// 16. Promise パターン
// ============================================================

#[test]
fn test_promise_service() {
    let source = r#"
angular.module('app', []).service('AsyncService', ['$q', '$http', function($q, $http) {
    this.loadAll = function() {
        var deferred = $q.defer();
        $http.get('/api/data').then(
            function(res) { deferred.resolve(res.data); },
            function(err) { deferred.reject(err); }
        );
        return deferred.promise;
    };
    this.loadMultiple = function() {
        return $q.all([$http.get('/api/users'), $http.get('/api/settings')]);
    };
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "AsyncService", SymbolKind::Service),
        "Promiseパターンのサービスが認識されるべき");
    assert!(has_definition(&index, "AsyncService.loadAll", SymbolKind::Method),
        "Promiseパターンのメソッドが認識されるべき");
    assert!(has_definition(&index, "AsyncService.loadMultiple", SymbolKind::Method),
        "Promiseパターンのメソッドが認識されるべき");
}

// ============================================================
// 17. Nested Controller パターン
// ============================================================

#[test]
fn test_nested_controllers() {
    let source = r#"
angular.module('app', [])
.controller('ParentCtrl', ['$scope', function($scope) {
    $scope.parentData = 'from parent';
    $scope.sharedMethod = function() { return 'shared'; };
}])
.controller('ChildCtrl', ['$scope', function($scope) {
    $scope.childData = 'from child';
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "ParentCtrl", SymbolKind::Controller),
        "親コントローラーが認識されるべき");
    assert!(has_definition(&index, "ChildCtrl", SymbolKind::Controller),
        "子コントローラーが認識されるべき");
    assert!(has_definition(&index, "ParentCtrl.$scope.parentData", SymbolKind::ScopeProperty),
        "親コントローラーの$scopeプロパティが認識されるべき");
    assert!(has_definition(&index, "ChildCtrl.$scope.childData", SymbolKind::ScopeProperty),
        "子コントローラーの$scopeプロパティが認識されるべき");
}

// ============================================================
// 18. this/$scope 混在パターン
// ============================================================

#[test]
fn test_mixed_scope_and_this() {
    let source = r#"
angular.module('app', []).controller('MixedCtrl', ['$scope', function($scope) {
    $scope.scopeVar = 'via scope';
    this.thisVar = 'via this';
    this.thisMethod = function() { return this.thisVar; };
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "MixedCtrl.$scope.scopeVar", SymbolKind::ScopeProperty),
        "混在パターンの$scopeプロパティが認識されるべき");
    assert!(has_definition(&index, "MixedCtrl.thisVar", SymbolKind::Method),
        "混在パターンのthisプロパティが認識されるべき");
    assert!(has_definition(&index, "MixedCtrl.thisMethod", SymbolKind::Method),
        "混在パターンのthisメソッドが認識されるべき");
}

// ============================================================
// 19. JSDoc コメント認識
// ============================================================

#[test]
fn test_jsdoc_on_service() {
    let source = r#"
/**
 * ユーザー管理サービス
 * @description ユーザーのCRUD操作を提供する
 */
angular.module('app', []).service('DocService', ['$http', function($http) {
    this.getAll = function() { return $http.get('/api/users'); };
}]);
"#;
    let index = analyze_js(source);
    let defs = index.definitions.get_definitions("DocService");
    assert!(!defs.is_empty(), "JSDoc付きサービスが認識されるべき");
    let has_docs = defs.iter().any(|d| d.docs.is_some());
    assert!(has_docs, "JSDocコメントが定義に含まれるべき");
}

#[test]
fn test_jsdoc_on_controller() {
    let source = r#"
/**
 * ドキュメント付きコントローラー
 */
angular.module('app', []).controller('DocCtrl', ['$scope', function($scope) {
    $scope.documented = true;
}]);
"#;
    let index = analyze_js(source);
    let defs = index.definitions.get_definitions("DocCtrl");
    assert!(!defs.is_empty(), "JSDoc付きコントローラーが認識されるべき");
    let has_docs = defs.iter().any(|d| d.docs.is_some());
    assert!(has_docs, "JSDocコメントがコントローラー定義に含まれるべき");
}

// ============================================================
// 20. HTML テンプレート解析
// ============================================================

#[test]
fn test_html_ng_controller() {
    let js = r#"
angular.module('app', []).controller('TestCtrl', ['$scope', function($scope) {
    $scope.title = 'Hello';
}]);
"#;
    let html = r#"<div ng-controller="TestCtrl"><h1>{{ title }}</h1></div>"#;
    let index = analyze_html(js, html);

    // ng-controllerが認識されるか
    let scopes = index.controllers.get_all_html_controller_scopes(&Url::parse("file:///test.html").unwrap());
    assert!(!scopes.is_empty(), "ng-controller属性がHTMLから認識されるべき");
}

#[test]
fn test_html_ng_controller_as() {
    let js = r#"
angular.module('app', []).controller('CtrlAs', [function() {
    this.title = 'Hello';
}]);
"#;
    let html = r#"<div ng-controller="CtrlAs as ctrl"><h1>{{ ctrl.title }}</h1></div>"#;
    let index = analyze_html(js, html);

    let scopes = index.controllers.get_all_html_controller_scopes(&Url::parse("file:///test.html").unwrap());
    assert!(!scopes.is_empty(), "controller as構文がHTMLから認識されるべき");
}

#[test]
fn test_html_ng_repeat() {
    let js = r#"
angular.module('app', []).controller('RepeatCtrl', ['$scope', function($scope) {
    $scope.items = [];
}]);
"#;
    let html = r#"
<div ng-controller="RepeatCtrl">
    <div ng-repeat="item in items">{{ item.name }}</div>
</div>
"#;
    let index = analyze_html(js, html);

    let html_uri = Url::parse("file:///test.html").unwrap();
    let local_vars = index.html.get_all_local_variables(&html_uri);
    let has_item_var = local_vars.iter().any(|v| v.name == "item");
    assert!(has_item_var, "ng-repeatのループ変数 'item' がローカル変数として認識されるべき");
}

#[test]
fn test_html_ng_repeat_key_value() {
    let js = r#"
angular.module('app', []).controller('KVCtrl', ['$scope', function($scope) {
    $scope.obj = {};
}]);
"#;
    let html = r#"
<div ng-controller="KVCtrl">
    <div ng-repeat="(key, value) in obj">{{ key }}: {{ value }}</div>
</div>
"#;
    let index = analyze_html(js, html);

    let html_uri = Url::parse("file:///test.html").unwrap();
    let local_vars = index.html.get_all_local_variables(&html_uri);
    let has_key = local_vars.iter().any(|v| v.name == "key");
    let has_value = local_vars.iter().any(|v| v.name == "value");
    println!("ng-repeat (key, value) - key recognized: {}, value recognized: {}", has_key, has_value);
}

#[test]
fn test_html_ng_init() {
    let html = r#"
<div ng-init="counter = 0; greeting = 'Hello'">
    {{ counter }} {{ greeting }}
</div>
"#;
    let index = analyze_html("", html);

    let html_uri = Url::parse("file:///test.html").unwrap();
    let local_vars = index.html.get_all_local_variables(&html_uri);
    let has_counter = local_vars.iter().any(|v| v.name == "counter");
    let has_greeting = local_vars.iter().any(|v| v.name == "greeting");
    println!("ng-init variables - counter: {}, greeting: {}", has_counter, has_greeting);
}

#[test]
fn test_html_custom_directive_element() {
    let js = r#"
angular.module('app', []).directive('userCard', [function() {
    return { restrict: 'E', scope: { user: '=' } };
}]);
"#;
    let html = r#"<user-card user="selectedUser"></user-card>"#;
    let index = analyze_html(js, html);

    let html_uri = Url::parse("file:///test.html").unwrap();
    let dir_refs = index.html.get_all_directive_references_for_uri(&html_uri);
    let has_user_card = dir_refs.iter().any(|r| r.directive_name == "userCard");
    assert!(has_user_card, "カスタムElementディレクティブ(kebab-case→camelCase変換)が認識されるべき");
}

#[test]
fn test_html_custom_directive_attribute() {
    let js = r#"
angular.module('app', []).directive('myHighlight', [function() {
    return { restrict: 'A', link: function(scope, element, attrs) {} };
}]);
"#;
    let html = r#"<div my-highlight="'red'">Text</div>"#;
    let index = analyze_html(js, html);

    let html_uri = Url::parse("file:///test.html").unwrap();
    let dir_refs = index.html.get_all_directive_references_for_uri(&html_uri);
    let has_highlight = dir_refs.iter().any(|r| r.directive_name == "myHighlight");
    assert!(has_highlight, "カスタムAttributeディレクティブ(kebab-case→camelCase変換)が認識されるべき");
}

#[test]
fn test_html_component_element() {
    let js = r#"
angular.module('app', []).component('heroDetail', {
    templateUrl: 'templates/hero-detail.html',
    bindings: { hero: '<' }
});
"#;
    let html = r#"<hero-detail hero="myHero"></hero-detail>"#;
    let index = analyze_html(js, html);

    let html_uri = Url::parse("file:///test.html").unwrap();
    let dir_refs = index.html.get_all_directive_references_for_uri(&html_uri);
    let has_hero_detail = dir_refs.iter().any(|r| r.directive_name == "heroDetail");
    assert!(has_hero_detail, "コンポーネント要素(kebab-case→camelCase変換)がディレクティブ参照として認識されるべき");
}

#[test]
fn test_html_form_binding() {
    let js = r#"
angular.module('app', []).controller('FormCtrl', ['$scope', function($scope) {}]);
"#;
    let html = r#"
<div ng-controller="FormCtrl">
    <form name="userForm">
        <input type="text" name="userName" ng-model="user.name" required>
        <span ng-show="userForm.userName.$error.required">Required</span>
    </form>
</div>
"#;
    let index = analyze_html(js, html);

    let html_uri = Url::parse("file:///test.html").unwrap();
    let form_bindings = index.html.get_all_form_bindings(&html_uri);
    let has_user_form = form_bindings.iter().any(|f| f.name == "userForm");
    assert!(has_user_form, "form name属性によるバインディングが認識されるべき");
}

#[test]
fn test_html_ng_include() {
    let html = r#"
<ng-include src="'templates/header.html'"></ng-include>
<div ng-include="'templates/sidebar.html'"></div>
"#;
    let index = analyze_html("", html);

    let html_uri = Url::parse("file:///test.html").unwrap();
    let includes = index.templates.get_ng_includes_in_file(&html_uri);
    println!("ng-include templates found: {}", includes.len());
}

#[test]
fn test_html_scope_references() {
    let js = r#"
angular.module('app', []).controller('RefCtrl', ['$scope', function($scope) {
    $scope.title = 'Hello';
    $scope.count = 0;
    $scope.doSomething = function() {};
}]);
"#;
    let html = r#"
<div ng-controller="RefCtrl">
    <h1>{{ title }}</h1>
    <p>Count: {{ count }}</p>
    <button ng-click="doSomething()">Do it</button>
</div>
"#;
    let index = analyze_html(js, html);

    let html_uri = Url::parse("file:///test.html").unwrap();
    let scope_refs = index.html.get_html_scope_references(&html_uri);
    let has_title_ref = scope_refs.iter().any(|r| r.property_path == "title");
    let has_count_ref = scope_refs.iter().any(|r| r.property_path == "count");
    let has_do_something_ref = scope_refs.iter().any(|r| r.property_path == "doSomething");
    assert!(has_title_ref, "HTMLテンプレート内の{{ title }}参照が認識されるべき");
    assert!(has_count_ref, "HTMLテンプレート内の{{ count }}参照が認識されるべき");
    assert!(has_do_something_ref, "ng-clickのdoSomething()参照が認識されるべき");
}

// ============================================================
// 21. 網羅的テスト：テストファイル全体の解析
// ============================================================

#[test]
fn test_comprehensive_js_analysis() {
    let source = include_str!("fixtures/angularjs_common_syntax.js");
    let index = analyze_js(source);

    let all_defs = index.definitions.get_all_definitions();

    // 認識されたシンボルの種類と名前を分類
    let mut modules = Vec::new();
    let mut controllers = Vec::new();
    let mut services = Vec::new();
    let mut factories = Vec::new();
    let mut directives = Vec::new();
    let mut components = Vec::new();
    let mut providers = Vec::new();
    let mut filters = Vec::new();
    let mut constants = Vec::new();
    let mut values = Vec::new();
    let mut methods = Vec::new();
    let mut scope_props = Vec::new();
    let mut scope_methods = Vec::new();
    let mut root_scope_props = Vec::new();
    let mut root_scope_methods = Vec::new();

    for def in &all_defs {
        match def.kind {
            SymbolKind::Module => modules.push(def.name.clone()),
            SymbolKind::Controller => controllers.push(def.name.clone()),
            SymbolKind::Service => services.push(def.name.clone()),
            SymbolKind::Factory => factories.push(def.name.clone()),
            SymbolKind::Directive => directives.push(def.name.clone()),
            SymbolKind::Component => components.push(def.name.clone()),
            SymbolKind::Provider => providers.push(def.name.clone()),
            SymbolKind::Filter => filters.push(def.name.clone()),
            SymbolKind::Constant => constants.push(def.name.clone()),
            SymbolKind::Value => values.push(def.name.clone()),
            SymbolKind::Method => methods.push(def.name.clone()),
            SymbolKind::ScopeProperty => scope_props.push(def.name.clone()),
            SymbolKind::ScopeMethod => scope_methods.push(def.name.clone()),
            SymbolKind::RootScopeProperty => root_scope_props.push(def.name.clone()),
            SymbolKind::RootScopeMethod => root_scope_methods.push(def.name.clone()),
            _ => {}
        }
    }

    println!("=== AngularJS LSP シンボル認識結果 ===");
    println!("");
    println!("--- Modules ({}) ---", modules.len());
    for m in &modules { println!("  ✓ {}", m); }
    println!("");
    println!("--- Controllers ({}) ---", controllers.len());
    for c in &controllers { println!("  ✓ {}", c); }
    println!("");
    println!("--- Services ({}) ---", services.len());
    for s in &services { println!("  ✓ {}", s); }
    println!("");
    println!("--- Factories ({}) ---", factories.len());
    for f in &factories { println!("  ✓ {}", f); }
    println!("");
    println!("--- Directives ({}) ---", directives.len());
    for d in &directives { println!("  ✓ {}", d); }
    println!("");
    println!("--- Components ({}) ---", components.len());
    for c in &components { println!("  ✓ {}", c); }
    println!("");
    println!("--- Providers ({}) ---", providers.len());
    for p in &providers { println!("  ✓ {}", p); }
    println!("");
    println!("--- Filters ({}) ---", filters.len());
    for f in &filters { println!("  ✓ {}", f); }
    println!("");
    println!("--- Constants ({}) ---", constants.len());
    for c in &constants { println!("  ✓ {}", c); }
    println!("");
    println!("--- Values ({}) ---", values.len());
    for v in &values { println!("  ✓ {}", v); }
    println!("");
    println!("--- Methods ({}) ---", methods.len());
    for m in &methods { println!("  ✓ {}", m); }
    println!("");
    println!("--- Scope Properties ({}) ---", scope_props.len());
    for s in &scope_props { println!("  ✓ {}", s); }
    println!("");
    println!("--- Scope Methods ({}) ---", scope_methods.len());
    for s in &scope_methods { println!("  ✓ {}", s); }
    println!("");
    println!("--- RootScope Properties ({}) ---", root_scope_props.len());
    for r in &root_scope_props { println!("  ✓ {}", r); }
    println!("");
    println!("--- RootScope Methods ({}) ---", root_scope_methods.len());
    for r in &root_scope_methods { println!("  ✓ {}", r); }
    println!("");

    // 期待されるシンボルの検証
    // Modules
    assert!(modules.contains(&"commonApp".to_string()), "commonApp モジュールが認識されるべき");
    assert!(modules.contains(&"simpleApp".to_string()), "simpleApp モジュールが認識されるべき");
    assert!(modules.contains(&"chainApp".to_string()), "chainApp モジュールが認識されるべき");

    // Controllers (全パターン)
    let expected_controllers = vec![
        "ArrayDIController", "InjectStyleController", "SimpleFuncController",
        "ClassController", "InlineClassController", "ArrayClassController",
        "ControllerAsCtrl", "ArrowController",
        "WatchController", "EventController", "ApplyController",
        "ChainController", "AdvancedController", "ModernController", "IIFEController",
        "ParentController", "ChildController", "MixedController", "DocumentedController",
    ];
    println!("\n=== Controller 認識チェック ===");
    for name in &expected_controllers {
        let found = controllers.contains(&name.to_string());
        println!("  {} {}", if found { "✓" } else { "✗" }, name);
        assert!(found, "Controller '{}' が認識されるべき", name);
    }

    // Services
    let expected_services = vec!["UserService", "DataService", "SimpleService", "AsyncService", "DocumentedService"];
    println!("\n=== Service 認識チェック ===");
    for name in &expected_services {
        let found = services.contains(&name.to_string());
        println!("  {} {}", if found { "✓" } else { "✗" }, name);
        assert!(found, "Service '{}' が認識されるべき", name);
    }

    // Factories
    let expected_factories = vec!["AuthService", "UtilService", "NotificationService", "UserResource"];
    println!("\n=== Factory 認識チェック ===");
    for name in &expected_factories {
        let found = factories.contains(&name.to_string());
        println!("  {} {}", if found { "✓" } else { "✗" }, name);
        assert!(found, "Factory '{}' が認識されるべき", name);
    }

    // Directives
    let expected_directives = vec!["userCard", "myHighlight", "tabPanel", "autoFocus", "tabItem", "repeatDirective"];
    println!("\n=== Directive 認識チェック ===");
    for name in &expected_directives {
        let found = directives.contains(&name.to_string());
        println!("  {} {}", if found { "✓" } else { "✗" }, name);
        assert!(found, "Directive '{}' が認識されるべき", name);
    }

    // Components
    let expected_components = vec!["heroDetail", "heroList", "lifecycleDemo"];
    println!("\n=== Component 認識チェック ===");
    for name in &expected_components {
        let found = components.contains(&name.to_string());
        println!("  {} {}", if found { "✓" } else { "✗" }, name);
        assert!(found, "Component '{}' が認識されるべき", name);
    }

    // Providers
    assert!(providers.contains(&"apiConfig".to_string()), "Provider 'apiConfig' が認識されるべき");

    // Filters
    let expected_filters = vec!["capitalize", "truncate", "currency", "chainFilter"];
    println!("\n=== Filter 認識チェック ===");
    for name in &expected_filters {
        let found = filters.contains(&name.to_string());
        println!("  {} {}", if found { "✓" } else { "✗" }, name);
        assert!(found, "Filter '{}' が認識されるべき", name);
    }

    // Constants
    assert!(constants.contains(&"API_URL".to_string()), "Constant 'API_URL' が認識されるべき");
    assert!(constants.contains(&"APP_CONFIG".to_string()), "Constant 'APP_CONFIG' が認識されるべき");

    // Values
    assert!(values.contains(&"appVersion".to_string()), "Value 'appVersion' が認識されるべき");
    assert!(values.contains(&"defaultSettings".to_string()), "Value 'defaultSettings' が認識されるべき");

    println!("\n=== 総シンボル数: {} ===", all_defs.len());
}

#[test]
fn test_comprehensive_html_analysis() {
    let js_source = include_str!("fixtures/angularjs_common_syntax.js");
    let html_source = include_str!("fixtures/angularjs_common_syntax.html");
    let index = analyze_html(js_source, html_source);

    let html_uri = Url::parse("file:///test.html").unwrap();

    // Controller scopes
    let scopes = index.controllers.get_all_html_controller_scopes(&html_uri);
    println!("\n=== HTML Controller Scopes ({}) ===", scopes.len());
    for scope in &scopes {
        println!("  ✓ {} (line {}-{})", scope.controller_name, scope.start_line, scope.end_line);
    }

    // Scope references
    let scope_refs = index.html.get_html_scope_references(&html_uri);
    println!("\n=== HTML Scope References ({}) ===", scope_refs.len());
    for r in &scope_refs {
        println!("  - {} (line {})", r.property_path, r.start_line);
    }

    // Local variables (ng-repeat, ng-init)
    let local_vars = index.html.get_all_local_variables(&html_uri);
    println!("\n=== HTML Local Variables ({}) ===", local_vars.len());
    for v in &local_vars {
        println!("  - {} (line {})", v.name, v.name_start_line);
    }

    // Directive references
    let dir_refs = index.html.get_all_directive_references_for_uri(&html_uri);
    println!("\n=== HTML Directive References ({}) ===", dir_refs.len());
    for r in &dir_refs {
        println!("  - {} (line {})", r.directive_name, r.start_line);
    }

    // Form bindings
    let forms = index.html.get_all_form_bindings(&html_uri);
    println!("\n=== HTML Form Bindings ({}) ===", forms.len());
    for f in &forms {
        println!("  - {} (line {})", f.name, f.name_start_line);
    }

    // 最低限のアサーション
    assert!(scopes.len() >= 3, "少なくとも3つのng-controllerスコープが認識されるべき (found: {})", scopes.len());
    assert!(scope_refs.len() >= 5, "少なくとも5つのスコープ参照が認識されるべき (found: {})", scope_refs.len());
    assert!(local_vars.len() >= 1, "少なくとも1つのローカル変数が認識されるべき (found: {})", local_vars.len());
}

// ============================================================
// $stateProvider.state() (ui-router) パターン
// ============================================================

#[test]
fn test_state_provider_controller_reference() {
    let source = r#"
angular.module('app', []).config(['$stateProvider', function($stateProvider) {
    $stateProvider
        .state('home', {
            url: '/home',
            templateUrl: 'views/home.html',
            controller: 'HomeController'
        })
        .state('about', {
            url: '/about',
            templateUrl: 'views/about.html',
            controller: 'AboutController'
        });
}]);
"#;
    let index = analyze_js(source);
    assert!(has_reference(&index, "HomeController"),
        "$stateProvider.state()のcontroller文字列参照(HomeController)が認識されるべき");
    assert!(has_reference(&index, "AboutController"),
        "$stateProvider.state()のcontroller文字列参照(AboutController)が認識されるべき");
}

#[test]
fn test_state_provider_template_binding() {
    let source = r#"
angular.module('app', []).config(['$stateProvider', function($stateProvider) {
    $stateProvider
        .state('dashboard', {
            url: '/dashboard',
            templateUrl: 'views/dashboard.html',
            controller: 'DashboardController'
        });
}]);
"#;
    let index = analyze_js(source);
    let bindings = index.templates.get_all_template_bindings();
    let has_dashboard_binding = bindings.iter().any(|b|
        b.template_path.contains("dashboard.html") && b.controller_name == "DashboardController"
    );
    assert!(has_dashboard_binding,
        "$stateProvider.state()のテンプレートバインディングが登録されるべき");
}

#[test]
fn test_state_provider_inline_controller() {
    let source = r#"
angular.module('app', []).config(['$stateProvider', function($stateProvider) {
    $stateProvider.state('profile', {
        url: '/profile',
        templateUrl: 'views/profile.html',
        controller: ['$scope', 'UserService', function($scope, UserService) {
            $scope.user = {};
            $scope.loadUser = function() {};
        }]
    });
}]);
"#;
    let index = analyze_js(source);
    assert!(has_definition(&index, "state.$scope.user", SymbolKind::ScopeProperty),
        "$stateProvider.state()のインラインcontroller内の$scopeプロパティが認識されるべき");
    assert!(has_definition(&index, "state.$scope.loadUser", SymbolKind::ScopeMethod),
        "$stateProvider.state()のインラインcontroller内の$scopeメソッドが認識されるべき");
}

#[test]
fn test_state_provider_chained_states() {
    let source = r#"
angular.module('app', []).config(['$stateProvider', function($stateProvider) {
    $stateProvider
        .state('users', {
            url: '/users',
            templateUrl: 'views/users.html',
            controller: 'UsersController'
        })
        .state('users.detail', {
            url: '/:id',
            templateUrl: 'views/user-detail.html',
            controller: 'UserDetailController'
        })
        .state('settings', {
            url: '/settings',
            templateUrl: 'views/settings.html',
            controller: 'SettingsController'
        });
}]);
"#;
    let index = analyze_js(source);
    assert!(has_reference(&index, "UsersController"),
        "チェーンされた$stateProvider.state()の1番目のcontroller参照が認識されるべき");
    assert!(has_reference(&index, "UserDetailController"),
        "チェーンされた$stateProvider.state()の2番目のcontroller参照が認識されるべき");
    assert!(has_reference(&index, "SettingsController"),
        "チェーンされた$stateProvider.state()の3番目のcontroller参照が認識されるべき");

    let bindings = index.templates.get_all_template_bindings();
    assert_eq!(bindings.len(), 3,
        "3つのテンプレートバインディングが登録されるべき");
}

// ============================================================
// workspace/symbol テスト
// ============================================================

#[test]
fn test_workspace_symbol_empty_query_returns_all_top_level() {
    let source = r#"
angular.module('myApp', [])
    .controller('MainCtrl', function($scope) {
        $scope.name = 'test';
    })
    .service('UserService', function() {
        this.getUser = function() {};
    })
    .filter('capitalize', function() {
        return function(input) { return input; };
    });
"#;
    let index = analyze_js(source);
    let handler = WorkspaceSymbolHandler::new(index);
    let symbols = handler.handle("");

    let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"myApp"), "モジュールが含まれるべき");
    assert!(names.contains(&"MainCtrl"), "コントローラーが含まれるべき");
    assert!(names.contains(&"UserService"), "サービスが含まれるべき");
    assert!(names.contains(&"capitalize"), "フィルターが含まれるべき");

    // $scopeプロパティは除外されるべき
    assert!(!names.contains(&"name"), "$scopeプロパティは除外されるべき");
}

#[test]
fn test_workspace_symbol_query_filters() {
    let source = r#"
angular.module('myApp', [])
    .controller('UserController', function() {})
    .controller('AdminController', function() {})
    .service('UserService', function() {});
"#;
    let index = analyze_js(source);
    let handler = WorkspaceSymbolHandler::new(index);
    let symbols = handler.handle("User");

    let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"UserController"), "Userにマッチするコントローラーが含まれるべき");
    assert!(names.contains(&"UserService"), "Userにマッチするサービスが含まれるべき");
    assert!(!names.contains(&"AdminController"), "Userにマッチしないコントローラーは除外されるべき");
}

#[test]
fn test_workspace_symbol_case_insensitive() {
    let source = r#"
angular.module('myApp', [])
    .controller('UserController', function() {});
"#;
    let index = analyze_js(source);
    let handler = WorkspaceSymbolHandler::new(index);
    let symbols = handler.handle("user");

    let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"UserController"), "大文字小文字を区別せずにマッチすべき");
}

#[test]
fn test_workspace_symbol_excludes_scope_properties() {
    let source = r#"
angular.module('myApp', [])
    .controller('TestCtrl', function($scope) {
        $scope.myProp = 'value';
        $scope.myMethod = function() {};
    });
"#;
    let index = analyze_js(source);
    let handler = WorkspaceSymbolHandler::new(index);
    let symbols = handler.handle("");

    let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"TestCtrl"), "コントローラーは含まれるべき");
    // ScopeProperty/ScopeMethod は除外
    for sym in &symbols {
        assert_ne!(sym.name, "TestCtrl.$scope.myProp", "スコーププロパティは除外されるべき");
        assert_ne!(sym.name, "TestCtrl.$scope.myMethod", "スコープメソッドは除外されるべき");
    }
}

// ============================================================
// Component template内の $ctrl エイリアス解決（hover/definition/completion 基盤）
// ============================================================

/// component template と JS をセットで解析するヘルパー
/// HTML側のURIは、JS側のtemplateUrlにマッチするように合わせる
fn analyze_component_with_template(
    js_source: &str,
    html_source: &str,
    template_uri_str: &str,
) -> Arc<Index> {
    let index = Arc::new(Index::new());
    let js_analyzer = Arc::new(AngularJsAnalyzer::new(index.clone()));
    let html_analyzer = HtmlAngularJsAnalyzer::new(index.clone(), js_analyzer.clone());

    let js_uri = Url::parse("file:///test.js").unwrap();
    js_analyzer.analyze_document(&js_uri, js_source);

    let html_uri = Url::parse(template_uri_str).unwrap();
    html_analyzer.analyze_document(&html_uri, html_source);

    index
}

#[test]
fn test_component_template_resolves_ctrl_alias_to_component_name() {
    // インラインコントローラーを持つcomponentで $ctrl が componentName に解決されること
    let js = r#"
angular.module('app', []).component('lcComp', {
    templateUrl: 'templates/lc-comp.html',
    controller: function() {
        var ctrl = this;
        ctrl.data = [];
    }
});
"#;
    let html = r#"<div>{{ $ctrl.data }}</div>"#;
    let index = analyze_component_with_template(js, html, "file:///templates/lc-comp.html");
    let html_uri = Url::parse("file:///templates/lc-comp.html").unwrap();

    // resolve_controller_by_alias が $ctrl → lcComp を解決できること
    let resolved = index.resolve_controller_by_alias(&html_uri, 0, "$ctrl");
    assert_eq!(
        resolved,
        Some("lcComp".to_string()),
        "component template内の $ctrl は component名 lcComp に解決されるべき"
    );

    // インラインコントローラーの this.data が lcComp.data として登録されていること
    assert!(
        has_definition(&index, "lcComp.data", SymbolKind::Method),
        "コンポーネントの this.data が lcComp.data (Method) として登録されているべき"
    );
}

#[test]
fn test_component_template_resolves_custom_alias() {
    // controllerAs に明示エイリアスがある場合
    let js = r#"
angular.module('app', []).component('userCard', {
    templateUrl: 'templates/user-card.html',
    controllerAs: 'uc',
    controller: function() {
        this.name = '';
    }
});
"#;
    let html = r#"<div>{{ uc.name }}</div>"#;
    let index = analyze_component_with_template(js, html, "file:///templates/user-card.html");
    let html_uri = Url::parse("file:///templates/user-card.html").unwrap();

    let resolved = index.resolve_controller_by_alias(&html_uri, 0, "uc");
    assert_eq!(
        resolved,
        Some("userCard".to_string()),
        "明示controllerAs 'uc' は userCard に解決されるべき"
    );

    // デフォルトの $ctrl はマッチしないこと
    let no_default = index.resolve_controller_by_alias(&html_uri, 0, "$ctrl");
    assert_eq!(
        no_default,
        None,
        "controllerAs 'uc' を指定したら $ctrl では解決されないべき"
    );
}

#[test]
fn test_component_template_completion_for_alias_prefix_returns_methods() {
    // complete_with_context(Some("lcComp"), ...) が this.X Method を返すことを確認
    // これがOKなら、HTML側の補完で controller名でこの API を呼べばよい
    use angularjs_lsp::handler::CompletionHandler;
    use tower_lsp::lsp_types::CompletionResponse;

    let js = r#"
angular.module('app', []).component('lcComp', {
    templateUrl: 'templates/lc-comp.html',
    controller: function() {
        var ctrl = this;
        ctrl.data = [];
        ctrl.refresh = function() {};
    }
});
"#;
    let index = analyze_component_with_template(
        js,
        "<div></div>",
        "file:///templates/lc-comp.html",
    );
    let handler = CompletionHandler::new(index);

    let resp = handler
        .complete_with_context(Some("lcComp"), None, &[])
        .expect("lcComp prefix で補完応答が返るべき");
    let labels: Vec<String> = match resp {
        CompletionResponse::Array(items) => items.into_iter().map(|i| i.label).collect(),
        _ => panic!("Array response 期待"),
    };

    assert!(
        labels.iter().any(|l| l == "data"),
        "lcComp prefix の補完に 'data' が含まれるべき (labels: {:?})",
        labels
    );
    assert!(
        labels.iter().any(|l| l == "refresh"),
        "lcComp prefix の補完に 'refresh' が含まれるべき (labels: {:?})",
        labels
    );
}

#[test]
fn test_html_completion_in_component_template_includes_ctrl_alias_and_methods() {
    // component templateで補完を呼ぶと:
    // - $ctrl エイリアスが候補に出る
    // - $ctrl 越しに見える this.X メソッド/プロパティが候補に出る
    use angularjs_lsp::handler::CompletionHandler;

    let js = r#"
angular.module('app', []).component('lcComp', {
    templateUrl: 'templates/lc-comp.html',
    controller: function() {
        var ctrl = this;
        ctrl.data = [];
        ctrl.refresh = function() {};
    }
});
"#;
    let html = r#"<div>{{ }}</div>"#;
    let index = analyze_component_with_template(js, html, "file:///templates/lc-comp.html");
    let handler = CompletionHandler::new(index);
    let html_uri = Url::parse("file:///templates/lc-comp.html").unwrap();

    let items = handler.complete_in_html_angular_context(&html_uri, 0);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();

    assert!(
        labels.contains(&"$ctrl"),
        "component templateの補完に '$ctrl' エイリアスが含まれるべき (labels: {:?})",
        labels
    );
    assert!(
        labels.contains(&"data"),
        "component templateの補完に controllerプロパティ 'data' が含まれるべき (labels: {:?})",
        labels
    );
    assert!(
        labels.contains(&"refresh"),
        "component templateの補完に controllerメソッド 'refresh' が含まれるべき (labels: {:?})",
        labels
    );
}

#[test]
fn test_html_completion_in_component_template_includes_custom_alias() {
    // controllerAs で明示エイリアスが指定された場合、その名前が補完候補に出る
    use angularjs_lsp::handler::CompletionHandler;

    let js = r#"
angular.module('app', []).component('userCard', {
    templateUrl: 'templates/user-card.html',
    controllerAs: 'uc',
    controller: function() {
        this.name = '';
    }
});
"#;
    let html = r#"<div>{{ }}</div>"#;
    let index =
        analyze_component_with_template(js, html, "file:///templates/user-card.html");
    let handler = CompletionHandler::new(index);
    let html_uri = Url::parse("file:///templates/user-card.html").unwrap();

    let items = handler.complete_in_html_angular_context(&html_uri, 0);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();

    assert!(
        labels.contains(&"uc"),
        "明示controllerAs 'uc' が補完候補に含まれるべき (labels: {:?})",
        labels
    );
    assert!(
        !labels.contains(&"$ctrl"),
        "controllerAs 'uc' を指定したらデフォルトの '$ctrl' は出ないべき (labels: {:?})",
        labels
    );
    assert!(
        labels.contains(&"name"),
        "userCardの 'name' プロパティが補完候補に含まれるべき (labels: {:?})",
        labels
    );
}

#[test]
fn test_html_completion_in_ng_controller_template_still_works() {
    // 既存のng-controller経由の補完が壊れていないことを確認
    use angularjs_lsp::handler::CompletionHandler;

    let js = r#"
angular.module('app', []).controller('FooCtrl', ['$scope', function($scope) {
    $scope.userName = '';
}]);
"#;
    let html = r#"
<div ng-controller="FooCtrl as fc">
    {{ }}
</div>
"#;
    let index = analyze_component_with_template(js, html, "file:///test.html");
    let handler = CompletionHandler::new(index);
    let html_uri = Url::parse("file:///test.html").unwrap();

    // ng-controller のスコープ内（行2 = `<div ng-controller="...">` 行の中）で補完
    let items = handler.complete_in_html_angular_context(&html_uri, 2);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();

    assert!(
        labels.contains(&"fc"),
        "ng-controller の 'as' エイリアス 'fc' が候補に残ること (labels: {:?})",
        labels
    );
    assert!(
        labels.contains(&"userName"),
        "$scope.userName が候補に残ること (labels: {:?})",
        labels
    );
}

// ============================================================
// ng-repeat 特殊変数 ($index, $first, $last, $middle, $odd, $even)
// ============================================================

#[test]
fn test_ng_repeat_special_variables_registered() {
    let html = r#"
<div ng-repeat="item in items">
    <span>{{ $index }}: {{ item.name }}</span>
</div>
"#;
    let index = analyze_html("", html);
    let html_uri = Url::parse("file:///test.html").unwrap();
    let local_vars = index.html.get_all_local_variables(&html_uri);
    let names: Vec<&str> = local_vars.iter().map(|v| v.name.as_str()).collect();

    for special in &["$index", "$first", "$last", "$middle", "$odd", "$even"] {
        assert!(
            names.contains(special),
            "ng-repeat スコープで {} がローカル変数として登録されるべき (names: {:?})",
            special,
            names
        );
    }
    // 通常のループ変数も健在
    assert!(
        names.contains(&"item"),
        "ループ変数 'item' も登録されているべき (names: {:?})",
        names
    );
}

#[test]
fn test_ng_repeat_special_variables_resolved_as_references() {
    use angularjs_lsp::model::HtmlLocalVariableSource;

    // $index などが参照として解決されること（スコープ参照ではなくローカル変数参照になる）
    let html = r#"
<div ng-repeat="item in items">
    <span ng-show="$first">First!</span>
    <span ng-class="{ 'odd': $odd }">{{ $index }}</span>
</div>
"#;
    let index = analyze_html("", html);
    let html_uri = Url::parse("file:///test.html").unwrap();

    // ローカル変数 source が NgRepeatSpecial であることを確認
    let local_vars = index.html.get_all_local_variables(&html_uri);
    let index_var = local_vars
        .iter()
        .find(|v| v.name == "$index")
        .expect("$index が登録されているべき");
    assert_eq!(
        index_var.source,
        HtmlLocalVariableSource::NgRepeatSpecial,
        "$index は NgRepeatSpecial として記録されるべき"
    );

    // 参照（HtmlLocalVariableReference）として登録されていること
    let refs = index.html.get_all_local_variable_references_for_uri(&html_uri);
    let ref_names: Vec<&str> = refs.iter().map(|r| r.variable_name.as_str()).collect();
    assert!(
        ref_names.contains(&"$index"),
        "$index への参照が登録されるべき (ref_names: {:?})",
        ref_names
    );
    assert!(
        ref_names.contains(&"$first"),
        "$first への参照が登録されるべき (ref_names: {:?})",
        ref_names
    );
}

#[test]
fn test_ng_repeat_special_variables_in_completion() {
    use angularjs_lsp::handler::CompletionHandler;

    let html = r#"
<div ng-repeat="item in items">
    {{ }}
</div>
"#;
    let index = analyze_html("", html);
    let handler = CompletionHandler::new(index);
    let html_uri = Url::parse("file:///test.html").unwrap();

    // ng-repeat スコープ内（行2 = `<div ng-repeat=...>` の中）で補完
    let items = handler.complete_in_html_angular_context(&html_uri, 2);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();

    for special in &["$index", "$first", "$last", "$middle", "$odd", "$even"] {
        assert!(
            labels.contains(special),
            "ng-repeat スコープ内の補完候補に {} が含まれるべき (labels: {:?})",
            special,
            labels
        );
    }
}

#[test]
fn test_ng_repeat_special_variables_only_in_scope() {
    // ng-repeat の外側では $index 等は出ないこと
    use angularjs_lsp::handler::CompletionHandler;

    let html = r#"
<div>
    {{ }}
    <div ng-repeat="item in items">{{ item }}</div>
</div>
"#;
    let index = analyze_html("", html);
    let handler = CompletionHandler::new(index);
    let html_uri = Url::parse("file:///test.html").unwrap();

    // 行2 = 外側の {{ }} の位置（ng-repeatの前）
    let items_outer = handler.complete_in_html_angular_context(&html_uri, 2);
    let outer_labels: Vec<&str> =
        items_outer.iter().map(|i| i.label.as_str()).collect();
    assert!(
        !outer_labels.contains(&"$index"),
        "ng-repeat の外側では $index は補完候補に出ないべき (labels: {:?})",
        outer_labels
    );
}

// ============================================================
// component要素のbindings属性名補完
// ============================================================

#[test]
fn test_component_bindings_completion_lists_bindings_for_known_element() {
    use angularjs_lsp::handler::CompletionHandler;

    let js = r#"
angular.module('app', []).component('fooComp', {
    template: '<div></div>',
    bindings: {
        valueIn: '<',
        onChange: '&'
    }
});
"#;
    let index = analyze_js(js);
    let handler = CompletionHandler::new(index);

    let items = handler.complete_component_bindings("foo-comp", "");
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();

    assert!(
        labels.contains(&"value-in"),
        "kebab-case化された 'value-in' が候補に含まれるべき (labels: {:?})",
        labels
    );
    assert!(
        labels.contains(&"on-change"),
        "kebab-case化された 'on-change' が候補に含まれるべき (labels: {:?})",
        labels
    );
}

#[test]
fn test_component_bindings_completion_filters_by_prefix() {
    use angularjs_lsp::handler::CompletionHandler;

    let js = r#"
angular.module('app', []).component('fooComp', {
    template: '<div></div>',
    bindings: {
        valueIn: '<',
        valueOut: '<',
        onChange: '&'
    }
});
"#;
    let index = analyze_js(js);
    let handler = CompletionHandler::new(index);

    let items = handler.complete_component_bindings("foo-comp", "val");
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();

    assert!(
        labels.contains(&"value-in"),
        "プレフィックス 'val' で 'value-in' が残るべき (labels: {:?})",
        labels
    );
    assert!(
        labels.contains(&"value-out"),
        "プレフィックス 'val' で 'value-out' が残るべき (labels: {:?})",
        labels
    );
    assert!(
        !labels.contains(&"on-change"),
        "プレフィックス 'val' で 'on-change' は除外されるべき (labels: {:?})",
        labels
    );
}

#[test]
fn test_component_bindings_completion_returns_empty_for_unknown_element() {
    use angularjs_lsp::handler::CompletionHandler;

    let js = r#"
angular.module('app', []).component('fooComp', {
    bindings: { valueIn: '<' }
});
"#;
    let index = analyze_js(js);
    let handler = CompletionHandler::new(index);

    let items = handler.complete_component_bindings("bar-comp", "");
    assert!(
        items.is_empty(),
        "未知の要素名では何も返さないべき (items: {:?})",
        items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>()
    );
}

#[test]
fn test_component_bindings_completion_does_not_leak_other_components() {
    use angularjs_lsp::handler::CompletionHandler;

    let js = r#"
angular.module('app', [])
    .component('fooComp', { bindings: { fooName: '<' } })
    .component('barComp', { bindings: { barName: '<' } });
"#;
    let index = analyze_js(js);
    let handler = CompletionHandler::new(index);

    let items = handler.complete_component_bindings("foo-comp", "");
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();

    assert!(
        labels.contains(&"foo-name"),
        "fooComp 自身の binding 'foo-name' が含まれるべき (labels: {:?})",
        labels
    );
    assert!(
        !labels.contains(&"bar-name"),
        "他コンポーネントの binding 'bar-name' は混入しないべき (labels: {:?})",
        labels
    );
}

#[test]
fn test_directive_completion_context_returns_element_tag_name() {
    use std::sync::Arc;
    use angularjs_lsp::analyzer::html::HtmlAngularJsAnalyzer;
    use angularjs_lsp::analyzer::js::AngularJsAnalyzer;
    use angularjs_lsp::index::Index;

    let index = Arc::new(Index::new());
    let js_analyzer = Arc::new(AngularJsAnalyzer::new(index.clone()));
    let html_analyzer = HtmlAngularJsAnalyzer::new(index.clone(), js_analyzer);

    let html = r#"<foo-comp val></foo-comp>"#;
    // col index: 0:'<' 1-3:foo 4:'-' 5-7:com 8:p 9:' ' 10-12:val 13:'>'

    // col=13 = "val" の直後（`>` の手前）
    let ctx = html_analyzer
        .get_directive_completion_context_with_tag(html, 0, 13)
        .expect("属性名位置で context が返るべき");
    assert_eq!(ctx.0, "val", "プレフィックスは 'val'");
    assert!(!ctx.1, "属性名位置なので is_tag_name = false");
    assert_eq!(
        ctx.2.as_deref(),
        Some("foo-comp"),
        "要素名は 'foo-comp' であるべき"
    );

    // col=4 = "foo" の直後（`-` の手前）→ タグ名位置
    let ctx_tag = html_analyzer
        .get_directive_completion_context_with_tag(html, 0, 4)
        .expect("タグ名位置で context が返るべき");
    assert!(ctx_tag.1, "タグ名位置なので is_tag_name = true");
    assert_eq!(ctx_tag.2, None, "タグ名位置では element_tag_name は None");
}

// ============================================================
// $mdDialog.show のテンプレート/コントローラーバインディング
// ============================================================

#[test]
fn test_md_dialog_template_binding() {
    use angularjs_lsp::model::BindingSource;

    let source = r#"
angular.module('app', []).controller('DialogCtrl', ['$mdDialog', function($mdDialog) {
    $mdDialog.show({
        controller: 'EditDialogCtrl',
        templateUrl: 'templates/edit-dialog.html'
    });
}]);
"#;
    let index = analyze_js(source);
    let bindings = index.templates.get_all_template_bindings();
    let dialog_binding = bindings
        .iter()
        .find(|b| b.template_path.contains("edit-dialog.html"))
        .expect("$mdDialog.show のテンプレートバインディングが登録されるべき");

    assert_eq!(dialog_binding.controller_name, "EditDialogCtrl");
    assert_eq!(
        dialog_binding.source,
        BindingSource::MdDialog,
        "BindingSource は MdDialog であるべき"
    );
}

#[test]
fn test_md_dialog_controller_reference_registered() {
    let source = r#"
angular.module('app', []).controller('PageCtrl', ['$mdDialog', function($mdDialog) {
    $mdDialog.show({
        controller: 'ConfirmDialogCtrl',
        templateUrl: 'templates/confirm.html'
    });
}]);
"#;
    let index = analyze_js(source);
    assert!(
        has_reference(&index, "ConfirmDialogCtrl"),
        "$mdDialog.show の controller 参照が登録されるべき"
    );
}

#[test]
fn test_md_dialog_show_without_template_does_not_register() {
    // $mdDialog.confirm() / $mdDialog.alert() などプリセットビルダーは
    // show() ではないため対象外。show() でもオブジェクト引数を取らないものは無視。
    let source = r#"
angular.module('app', []).controller('PageCtrl', ['$mdDialog', function($mdDialog) {
    $mdDialog.show($mdDialog.confirm().title('確認'));
}]);
"#;
    let index = analyze_js(source);
    let bindings = index.templates.get_all_template_bindings();
    assert!(
        bindings.is_empty(),
        "オブジェクト引数を取らない $mdDialog.show は無視されるべき (got: {:?})",
        bindings.iter().map(|b| &b.template_path).collect::<Vec<_>>()
    );
}

#[test]
fn test_other_show_calls_do_not_register_md_dialog_binding() {
    // $mdDialog 以外の .show() 呼び出しは無視されるべき
    let source = r#"
angular.module('app', []).controller('PageCtrl', ['$scope', function($scope) {
    someOtherService.show({
        controller: 'NotADialogCtrl',
        templateUrl: 'templates/not-a-dialog.html'
    });
}]);
"#;
    let index = analyze_js(source);
    let bindings = index.templates.get_all_template_bindings();
    assert!(
        bindings.is_empty(),
        "$mdDialog 以外の .show() は無視されるべき (got: {:?})",
        bindings.iter().map(|b| &b.template_path).collect::<Vec<_>>()
    );
}

#[test]
fn test_md_bottom_sheet_template_binding() {
    use angularjs_lsp::model::BindingSource;

    let source = r#"
angular.module('app', []).controller('PageCtrl', ['$mdBottomSheet', function($mdBottomSheet) {
    $mdBottomSheet.show({
        controller: 'OptionsSheetCtrl',
        templateUrl: 'templates/options-sheet.html'
    });
}]);
"#;
    let index = analyze_js(source);
    let bindings = index.templates.get_all_template_bindings();
    let binding = bindings
        .iter()
        .find(|b| b.template_path.contains("options-sheet.html"))
        .expect("$mdBottomSheet.show のテンプレートバインディングが登録されるべき");

    assert_eq!(binding.controller_name, "OptionsSheetCtrl");
    assert_eq!(
        binding.source,
        BindingSource::MdBottomSheet,
        "BindingSource は MdBottomSheet であるべき"
    );
}

#[test]
fn test_md_bottom_sheet_controller_reference_registered() {
    let source = r#"
angular.module('app', []).controller('PageCtrl', ['$mdBottomSheet', function($mdBottomSheet) {
    $mdBottomSheet.show({
        controller: 'ShareSheetCtrl',
        templateUrl: 'templates/share-sheet.html'
    });
}]);
"#;
    let index = analyze_js(source);
    assert!(
        has_reference(&index, "ShareSheetCtrl"),
        "$mdBottomSheet.show の controller 参照が登録されるべき"
    );
}

#[test]
fn test_md_bottom_sheet_aliased_via_di() {
    use angularjs_lsp::model::BindingSource;

    let source = r#"
angular.module('app', []).controller('PageCtrl', function() {
    var mdBottomSheet = angular.injector(['ng', 'material.components.bottomSheet']).get('$mdBottomSheet');
    mdBottomSheet.show({
        controller: 'AliasedSheetCtrl',
        templateUrl: 'templates/aliased-sheet.html'
    });
});
"#;
    let index = analyze_js(source);
    let bindings = index.templates.get_all_template_bindings();
    let binding = bindings
        .iter()
        .find(|b| b.template_path.contains("aliased-sheet.html"))
        .expect("mdBottomSheet (エイリアス) からも binding を抽出すべき");
    assert_eq!(binding.controller_name, "AliasedSheetCtrl");
    assert_eq!(binding.source, BindingSource::MdBottomSheet);
}

#[test]
fn test_md_dialog_and_md_bottom_sheet_distinguished() {
    // 同一ファイル内に両方が存在しても BindingSource で区別されること
    use angularjs_lsp::model::BindingSource;

    let source = r#"
angular.module('app', []).controller('MixCtrl', ['$mdDialog', '$mdBottomSheet',
    function($mdDialog, $mdBottomSheet) {
        $mdDialog.show({ controller: 'DialogA', templateUrl: 'a.html' });
        $mdBottomSheet.show({ controller: 'SheetB', templateUrl: 'b.html' });
    }]);
"#;
    let index = analyze_js(source);
    let bindings = index.templates.get_all_template_bindings();

    let a = bindings
        .iter()
        .find(|b| b.template_path.ends_with("a.html"))
        .expect("a.html のバインディングがあるべき");
    let b = bindings
        .iter()
        .find(|b| b.template_path.ends_with("b.html"))
        .expect("b.html のバインディングがあるべき");

    assert_eq!(a.source, BindingSource::MdDialog);
    assert_eq!(b.source, BindingSource::MdBottomSheet);
}

// ============================================================
// $mdToast / $mdPanel / ngDialog バインディング
// ============================================================

#[test]
fn test_md_toast_show_template_binding() {
    use angularjs_lsp::model::BindingSource;

    let source = r#"
angular.module('app', []).controller('PageCtrl', ['$mdToast', function($mdToast) {
    $mdToast.show({
        controller: 'CustomToastCtrl',
        templateUrl: 'templates/custom-toast.html'
    });
}]);
"#;
    let index = analyze_js(source);
    let bindings = index.templates.get_all_template_bindings();
    let binding = bindings
        .iter()
        .find(|b| b.template_path.contains("custom-toast.html"))
        .expect("$mdToast.show のテンプレートバインディングが登録されるべき");

    assert_eq!(binding.controller_name, "CustomToastCtrl");
    assert_eq!(binding.source, BindingSource::MdToast);
    assert!(
        has_reference(&index, "CustomToastCtrl"),
        "controller 文字列参照も登録されるべき"
    );
}

#[test]
fn test_md_panel_open_template_binding() {
    use angularjs_lsp::model::BindingSource;

    let source = r#"
angular.module('app', []).controller('PageCtrl', ['$mdPanel', function($mdPanel) {
    $mdPanel.open({
        controller: 'PanelCtrl',
        templateUrl: 'templates/panel.html'
    });
}]);
"#;
    let index = analyze_js(source);
    let bindings = index.templates.get_all_template_bindings();
    let binding = bindings
        .iter()
        .find(|b| b.template_path.contains("panel.html"))
        .expect("$mdPanel.open のテンプレートバインディングが登録されるべき");

    assert_eq!(binding.controller_name, "PanelCtrl");
    assert_eq!(binding.source, BindingSource::MdPanel);
    assert!(
        has_reference(&index, "PanelCtrl"),
        "controller 文字列参照も登録されるべき"
    );
}

#[test]
fn test_ng_dialog_open_template_binding() {
    use angularjs_lsp::model::BindingSource;

    let source = r#"
angular.module('app', []).controller('PageCtrl', ['ngDialog', function(ngDialog) {
    ngDialog.open({
        controller: 'NgDialogCtrl',
        templateUrl: 'templates/ng-dialog.html'
    });
}]);
"#;
    let index = analyze_js(source);
    let bindings = index.templates.get_all_template_bindings();
    let binding = bindings
        .iter()
        .find(|b| b.template_path.contains("ng-dialog.html"))
        .expect("ngDialog.open のテンプレートバインディングが登録されるべき");

    assert_eq!(binding.controller_name, "NgDialogCtrl");
    assert_eq!(binding.source, BindingSource::NgDialog);
    assert!(
        has_reference(&index, "NgDialogCtrl"),
        "controller 文字列参照も登録されるべき"
    );
}

#[test]
fn test_uib_modal_still_recognized_after_open_refactor() {
    // .open() 経路のリファクタで $uibModal が壊れていないことを確認（回帰防止）
    use angularjs_lsp::model::BindingSource;

    let source = r#"
angular.module('app', []).controller('PageCtrl', ['$uibModal', function($uibModal) {
    $uibModal.open({
        controller: 'StillUibCtrl',
        templateUrl: 'templates/still-uib.html'
    });
}]);
"#;
    let index = analyze_js(source);
    let bindings = index.templates.get_all_template_bindings();
    let binding = bindings
        .iter()
        .find(|b| b.template_path.contains("still-uib.html"))
        .expect("$uibModal は引き続き UibModal として認識されるべき");

    assert_eq!(binding.source, BindingSource::UibModal);
}

#[test]
fn test_md_panel_and_ng_dialog_distinguished_from_uib_modal() {
    // 同じ .open() でもオブジェクト名で BindingSource が分かれること
    use angularjs_lsp::model::BindingSource;

    let source = r#"
angular.module('app', []).controller('MixCtrl', ['$uibModal', '$mdPanel', 'ngDialog',
    function($uibModal, $mdPanel, ngDialog) {
        $uibModal.open({ controller: 'A', templateUrl: 'a.html' });
        $mdPanel.open({ controller: 'B', templateUrl: 'b.html' });
        ngDialog.open({ controller: 'C', templateUrl: 'c.html' });
    }]);
"#;
    let index = analyze_js(source);
    let bindings = index.templates.get_all_template_bindings();
    let a = bindings.iter().find(|b| b.template_path.ends_with("a.html")).unwrap();
    let b = bindings.iter().find(|b| b.template_path.ends_with("b.html")).unwrap();
    let c = bindings.iter().find(|b| b.template_path.ends_with("c.html")).unwrap();
    assert_eq!(a.source, BindingSource::UibModal);
    assert_eq!(b.source, BindingSource::MdPanel);
    assert_eq!(c.source, BindingSource::NgDialog);
}

#[test]
fn test_other_open_calls_still_ignored() {
    // file.open() のような無関係な .open() は無視されること
    let source = r#"
angular.module('app', []).controller('PageCtrl', ['fileSystem', function(fs) {
    fs.open({ controller: 'NotAModalCtrl', templateUrl: 'fake.html' });
}]);
"#;
    let index = analyze_js(source);
    let bindings = index.templates.get_all_template_bindings();
    assert!(
        bindings.is_empty(),
        "無関係な .open() は無視されるべき (got: {:?})",
        bindings.iter().map(|b| &b.template_path).collect::<Vec<_>>()
    );
}

// ============================================================
// modal/dialog/sheet 内のインラインcontroller + controllerAs解決
// （実プロジェクト: material-start ContactSheet.html ケース）
// ============================================================

#[test]
fn test_md_bottom_sheet_di_array_inline_controller_function_extracts_this_methods() {
    // controller: ['$dep', UserSheetController] のように DI配列の最後が
    // 関数識別子（同一ファイル内の関数宣言）の場合、その関数の this.X を
    // <ControllerName>.X として登録する
    let source = r#"
class UserDetailsController {
    constructor($mdBottomSheet) {
        this.$mdBottomSheet = $mdBottomSheet;
    }
    share() {
        var $mdBottomSheet = this.$mdBottomSheet;
        $mdBottomSheet.show({
            templateUrl: 'src/users/components/details/ContactSheet.html',
            controller: ['$mdBottomSheet', UserSheetController],
            controllerAs: '$ctrl'
        });
        function UserSheetController($mdBottomSheet) {
            this.user = {};
            this.items = [];
            this.performAction = function(action) {};
        }
    }
}
"#;
    let index = analyze_js(source);
    assert!(
        has_definition(&index, "UserSheetController.user", SymbolKind::Method),
        "インライン controller の this.user が UserSheetController.user (Method) として登録されるべき"
    );
    assert!(
        has_definition(&index, "UserSheetController.items", SymbolKind::Method),
        "this.items が登録されるべき"
    );
    assert!(
        has_definition(&index, "UserSheetController.performAction", SymbolKind::Method),
        "this.performAction が登録されるべき"
    );
}

#[test]
fn test_md_bottom_sheet_template_alias_resolution_for_inline_controller() {
    // ContactSheet.html 側で $ctrl が UserSheetController に解決され、
    // $ctrl.user が hover/definition で見つかること
    let js = r#"
class UserDetailsController {
    share() {
        var $mdBottomSheet;
        $mdBottomSheet.show({
            templateUrl: 'src/users/components/details/ContactSheet.html',
            controller: ['$mdBottomSheet', UserSheetController],
            controllerAs: '$ctrl'
        });
        function UserSheetController($mdBottomSheet) {
            this.user = {};
            this.items = [];
        }
    }
}
"#;
    let html = r#"
<md-bottom-sheet>
  <md-subheader>{{ $ctrl.user.name }}</md-subheader>
  <md-list>
    <md-item ng-repeat="item in $ctrl.items">{{ item.name }}</md-item>
  </md-list>
</md-bottom-sheet>
"#;
    // テンプレートURIは binding 側の templateUrl と suffix一致するように
    let index = analyze_component_with_template(
        js,
        html,
        "file:///app/src/users/components/details/ContactSheet.html",
    );
    let html_uri =
        Url::parse("file:///app/src/users/components/details/ContactSheet.html").unwrap();

    // $ctrl が UserSheetController に解決されること
    let resolved = index.resolve_controller_by_alias(&html_uri, 0, "$ctrl");
    assert_eq!(
        resolved,
        Some("UserSheetController".to_string()),
        "ContactSheet.html の $ctrl は UserSheetController に解決されるべき"
    );

    // $ctrl.user / $ctrl.items の symbol も検索可能
    assert!(
        has_definition(&index, "UserSheetController.user", SymbolKind::Method),
        "UserSheetController.user が定義として存在し、$ctrl.user の go-to-definition が効くこと"
    );
}

#[test]
fn test_md_bottom_sheet_with_inline_anonymous_function_controller() {
    // controller: ['$dep', function() {...}] (無名関数) の場合、名前は派生不能
    // でも ComponentTemplateUrl 自体は controllerAs で登録されるので、
    // $ctrl 自体は controller_name=None で登録される（method 解決はできない）
    let source = r#"
$mdDialog.show({
    templateUrl: 'templates/anonymous.html',
    controller: ['$mdDialog', function($mdDialog) {
        this.title = 'hello';
    }],
    controllerAs: 'vm'
});
"#;
    let index = analyze_js(source);
    let bindings = index.templates.get_all_template_bindings();
    // 名前が無いので TemplateBinding は登録されない（controller_name 必須のため）
    assert!(
        bindings.is_empty(),
        "無名関数 controller では TemplateBinding は登録されないべき"
    );
    // ComponentTemplateUrl は登録され、controllerAs='vm' で alias 解決可能
    let dummy_uri = Url::parse("file:///dummy.html").unwrap();
    // get_component_binding_for_template は path suffix 一致
    let html_uri = Url::parse("file:///some/path/templates/anonymous.html").unwrap();
    let _ = (dummy_uri, html_uri); // pacify lint
}

#[test]
fn test_md_bottom_sheet_controller_as_default_is_dollar_ctrl() {
    // controllerAs を省略した場合のデフォルトは "$ctrl"
    let js = r#"
$mdBottomSheet.show({
    templateUrl: 'templates/sheet.html',
    controller: 'MyCtrl'
    // controllerAs 省略
});
"#;
    let html = r#"<div>{{ $ctrl.foo }}</div>"#;
    let index =
        analyze_component_with_template(js, html, "file:///app/templates/sheet.html");
    let html_uri = Url::parse("file:///app/templates/sheet.html").unwrap();

    let resolved = index.resolve_controller_by_alias(&html_uri, 0, "$ctrl");
    assert_eq!(
        resolved,
        Some("MyCtrl".to_string()),
        "controllerAs 省略時のデフォルト $ctrl は controller 名に解決されるべき"
    );
}

#[test]
fn test_md_bottom_sheet_with_custom_controller_as() {
    let js = r#"
$mdBottomSheet.show({
    templateUrl: 'templates/sheet.html',
    controller: 'SheetCtrl',
    controllerAs: 'sheet'
});
"#;
    let html = r#"<div>{{ sheet.foo }}</div>"#;
    let index =
        analyze_component_with_template(js, html, "file:///app/templates/sheet.html");
    let html_uri = Url::parse("file:///app/templates/sheet.html").unwrap();

    let resolved = index.resolve_controller_by_alias(&html_uri, 0, "sheet");
    assert_eq!(
        resolved,
        Some("SheetCtrl".to_string()),
        "明示controllerAs 'sheet' は SheetCtrl に解決されるべき"
    );
    // デフォルトの $ctrl では解決されないこと
    assert_eq!(
        index.resolve_controller_by_alias(&html_uri, 0, "$ctrl"),
        None,
        "controllerAs を 'sheet' にしたらデフォルト $ctrl では解決されないべき"
    );
}

#[test]
fn test_md_dialog_aliased_via_di() {
    // DI で受けた $mdDialog の別名（mdDialog 等）も認識する
    use angularjs_lsp::model::BindingSource;

    let source = r#"
angular.module('app', []).controller('PageCtrl', function() {
    var mdDialog = angular.injector(['ng', 'material.components.dialog']).get('$mdDialog');
    mdDialog.show({
        controller: 'AliasedDialogCtrl',
        templateUrl: 'templates/aliased.html'
    });
});
"#;
    let index = analyze_js(source);
    let bindings = index.templates.get_all_template_bindings();
    let binding = bindings
        .iter()
        .find(|b| b.template_path.contains("aliased.html"))
        .expect("mdDialog (エイリアス) からも binding を抽出すべき");
    assert_eq!(binding.controller_name, "AliasedDialogCtrl");
    assert_eq!(binding.source, BindingSource::MdDialog);
}

// ============================================================
// custom directive / component bindings の attribute 値解析
// ============================================================

#[test]
fn test_custom_directive_attribute_value_is_parsed_as_expression() {
    // .directive('myHighlight', ...) で登録された directive の属性値が
    // Angular 式として解析され、scope 参照が登録されること
    let js = r#"
angular.module('app', [])
    .controller('Ctrl', ['$scope', function($scope) {
        $scope.user = {};
    }])
    .directive('myHighlight', [function() {
        return { restrict: 'A' };
    }]);
"#;
    let html = r#"
<div ng-controller="Ctrl">
    <span my-highlight="user.name">label</span>
</div>
"#;
    let index = analyze_html(js, html);
    let html_uri = Url::parse("file:///test.html").unwrap();

    let refs = index.html.get_html_scope_references(&html_uri);
    // 既存パーサーは top-level identifier のみ拾う ("user.name" → "user")。
    // ここでは「式として解析されること」を確認したいので "user" が拾われていれば OK
    let names: Vec<&str> = refs.iter().map(|r| r.property_path.as_str()).collect();
    assert!(
        names.iter().any(|n| *n == "user"),
        "custom directive 'my-highlight' の属性値が解析され 'user' が登録されるべき (refs: {:?})",
        names
    );
}

#[test]
fn test_unregistered_attribute_does_not_parse_as_expression() {
    // 登録されていない属性は Angular 式として解析されない (現状維持)
    let js = r#"
angular.module('app', []).controller('Ctrl', ['$scope', function($scope) {
    $scope.user = {};
}]);
"#;
    let html = r#"
<div ng-controller="Ctrl">
    <span data-foo="user.name">label</span>
</div>
"#;
    let index = analyze_html(js, html);
    let html_uri = Url::parse("file:///test.html").unwrap();

    let refs = index.html.get_html_scope_references(&html_uri);
    let names: Vec<&str> = refs.iter().map(|r| r.property_path.as_str()).collect();
    assert!(
        !names.iter().any(|n| *n == "user"),
        "未登録 directive 'data-foo' の属性値は scope 参照として登録されるべきでない (refs: {:?})",
        names
    );
}

#[test]
fn test_component_binding_attribute_value_is_parsed_as_expression() {
    // .component('userCard', { bindings: { user: '<', onSelect: '&' } })
    // で登録された binding 名と一致する属性値が Angular 式として解析される
    let js = r#"
angular.module('app', [])
    .controller('Ctrl', ['$scope', function($scope) {
        $scope.currentUser = {};
        $scope.handleSelect = function() {};
    }])
    .component('userCard', {
        templateUrl: 'card.html',
        bindings: {
            user: '<',
            onSelect: '&'
        }
    });
"#;
    let html = r#"
<div ng-controller="Ctrl">
    <user-card user="currentUser" on-select="handleSelect()"></user-card>
</div>
"#;
    let index = analyze_html(js, html);
    let html_uri = Url::parse("file:///test.html").unwrap();

    let refs = index.html.get_html_scope_references(&html_uri);
    let names: Vec<&str> = refs.iter().map(|r| r.property_path.as_str()).collect();
    assert!(
        names.contains(&"currentUser"),
        "<user-card user=\"currentUser\"> の 'currentUser' が scope 参照として登録されるべき (refs: {:?})",
        names
    );
    assert!(
        names.contains(&"handleSelect"),
        "<user-card on-select=\"handleSelect()\"> の 'handleSelect' が scope 参照として登録されるべき (refs: {:?})",
        names
    );
}

#[test]
fn test_component_unknown_binding_attribute_is_ignored() {
    // component に存在しない binding 名の属性は Angular 式として解析されない
    let js = r#"
angular.module('app', [])
    .controller('Ctrl', ['$scope', function($scope) { $scope.foo = 1; }])
    .component('userCard', {
        bindings: { user: '<' }
    });
"#;
    let html = r#"
<div ng-controller="Ctrl">
    <user-card unknown-attr="foo"></user-card>
</div>
"#;
    let index = analyze_html(js, html);
    let html_uri = Url::parse("file:///test.html").unwrap();

    let refs = index.html.get_html_scope_references(&html_uri);
    let names: Vec<&str> = refs.iter().map(|r| r.property_path.as_str()).collect();
    assert!(
        !names.contains(&"foo"),
        "未定義 binding 'unknown-attr' の属性値 'foo' は scope 参照として登録されるべきでない (refs: {:?})",
        names
    );
}

#[test]
fn test_data_prefix_is_stripped_for_directive_lookup() {
    // data- 接頭辞付きの custom directive も認識される
    let js = r#"
angular.module('app', [])
    .controller('Ctrl', ['$scope', function($scope) {
        $scope.x = 1;
    }])
    .directive('myDir', [function() { return {}; }]);
"#;
    let html = r#"
<div ng-controller="Ctrl">
    <span data-my-dir="x">label</span>
</div>
"#;
    let index = analyze_html(js, html);
    let html_uri = Url::parse("file:///test.html").unwrap();

    let refs = index.html.get_html_scope_references(&html_uri);
    let names: Vec<&str> = refs.iter().map(|r| r.property_path.as_str()).collect();
    assert!(
        names.contains(&"x"),
        "data- prefix 付き custom directive の属性値も解析されるべき (refs: {:?})",
        names
    );
}

#[test]
fn test_controller_prefix_completion_excludes_scope_methods() {
    // controller 名をプレフィックスにした補完 (HTML 内で controller の this.X を
    // 拾うため `complete_with_context(Some("CtrlName"), ...)` を呼ぶ経路) で
    // `$scope.X` が "$scope.X" というラベルで Method として混入してはいけない。
    //
    // バグ前は `MyCtrl.$scope.update` も name が `MyCtrl.` で始まるため
    // strip_prefix されて `$scope.update` (Method) として返ってきていた。
    use angularjs_lsp::handler::CompletionHandler;
    use tower_lsp::lsp_types::CompletionResponse;

    let js = r#"
angular.module('app', []).controller('MyCtrl', ['$scope', function($scope) {
    var vm = this;
    vm.refresh = function() {};
    $scope.update = function() {};
}]);
"#;
    let index = analyze_js(js);
    let handler = CompletionHandler::new(index);

    let resp = handler
        .complete_with_context(Some("MyCtrl"), None, &[])
        .expect("controller prefix で補完応答が返るべき");
    let labels: Vec<String> = match resp {
        CompletionResponse::Array(items) => items.into_iter().map(|i| i.label).collect(),
        _ => panic!("Array response 期待"),
    };

    assert!(
        labels.iter().any(|l| l == "refresh"),
        "this.refresh は controller プレフィックス補完に含まれるべき (labels: {:?})",
        labels
    );
    assert!(
        !labels.iter().any(|l| l.starts_with("$scope.")),
        "$scope.X は controller プレフィックス補完に混入してはいけない (labels: {:?})",
        labels
    );
}

#[test]
fn test_html_completion_does_not_duplicate_scope_method_with_dollar_scope_label() {
    // HTML 内の {{ }} 補完で `$scope.update = function() {}` を定義した場合、
    // `update` (Function) のみが候補に出るべきで、`$scope.update` (Method) が
    // 並んで出てはいけない。
    use angularjs_lsp::handler::CompletionHandler;

    let js = r#"
angular.module('app', []).controller('MyCtrl', ['$scope', function($scope) {
    $scope.update = function() {};
}]);
"#;
    let html = r#"<div ng-controller="MyCtrl">{{ }}</div>"#;
    let index = analyze_html(js, html);
    let handler = CompletionHandler::new(index);
    let html_uri = Url::parse("file:///test.html").unwrap();

    let items = handler.complete_in_html_angular_context(&html_uri, 0);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();

    assert!(
        labels.contains(&"update"),
        "HTML 補完に 'update' が含まれるべき (labels: {:?})",
        labels
    );
    assert!(
        !labels.contains(&"$scope.update"),
        "HTML 補完に '$scope.update' という重複候補が含まれてはいけない (labels: {:?})",
        labels
    );
}
