///! AngularJS一般的な構文のLSP対応状況を検証する統合テスト
///!
///! このテストは、AngularJS 1.xで使われる主要な構文パターンを網羅し、
///! LSPのアナライザーが各パターンを正しく認識できるか検証する。

use std::sync::Arc;
use tower_lsp::lsp_types::Url;

use angularjs_lsp::analyzer::js::AngularJsAnalyzer;
use angularjs_lsp::analyzer::html::HtmlAngularJsAnalyzer;
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
