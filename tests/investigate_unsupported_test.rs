///! AngularJS LSPが対応していない構文・機能を調査するテスト
///!
///! このテストは assert! ではなく println! で結果を出力し、
///! 対応・非対応の状況を可視化する。

use std::sync::Arc;
use tower_lsp::lsp_types::Url;

use angularjs_lsp::analyzer::html::HtmlAngularJsAnalyzer;
use angularjs_lsp::analyzer::js::AngularJsAnalyzer;
use angularjs_lsp::index::Index;
use angularjs_lsp::model::SymbolKind;

fn analyze_js(source: &str) -> Arc<Index> {
    let index = Arc::new(Index::new());
    let analyzer = AngularJsAnalyzer::new(index.clone());
    let uri = Url::parse("file:///test.js").unwrap();
    analyzer.analyze_document(&uri, source);
    index
}

fn analyze_html(js_source: &str, html_source: &str) -> Arc<Index> {
    let index = Arc::new(Index::new());
    let js_analyzer = Arc::new(AngularJsAnalyzer::new(index.clone()));
    let html_analyzer = HtmlAngularJsAnalyzer::new(index.clone(), js_analyzer.clone());
    if !js_source.is_empty() {
        let js_uri = Url::parse("file:///test.js").unwrap();
        js_analyzer.analyze_document(&js_uri, js_source);
    }
    let html_uri = Url::parse("file:///test.html").unwrap();
    html_analyzer.analyze_document(&html_uri, html_source);
    index
}

fn has_def(index: &Index, name: &str, kind: SymbolKind) -> bool {
    index
        .definitions
        .get_definitions(name)
        .iter()
        .any(|d| d.kind == kind)
}

fn has_any_def(index: &Index, name: &str) -> bool {
    !index.definitions.get_definitions(name).is_empty()
}

fn has_ref(index: &Index, name: &str) -> bool {
    !index.definitions.get_references(name).is_empty()
}

fn check(label: &str, result: bool) {
    println!("  {} {}", if result { "✓ [対応]" } else { "✗ [非対応]" }, label);
}

// ============================================================
// JS側の非対応調査
// ============================================================

#[test]
fn investigate_js_unsupported_patterns() {
    println!("\n========================================");
    println!("  AngularJS LSP 非対応構文・機能の調査");
    println!("  (JS側)");
    println!("========================================\n");

    // --- 1. decorator パターン ---
    println!("--- 1. module.decorator() パターン ---");
    {
        let source = r#"
angular.module('app', []).decorator('$log', ['$delegate', function($delegate) {
    return $delegate;
}]);
"#;
        let index = analyze_js(source);
        check("module.decorator() がシンボルとして認識される", has_any_def(&index, "$log"));
    }

    // --- 2. $resource カスタムアクション ---
    println!("\n--- 2. $resource カスタムアクション ---");
    {
        let source = r#"
angular.module('app', []).factory('UserResource', ['$resource', function($resource) {
    return $resource('/api/users/:id', { id: '@id' }, {
        update: { method: 'PUT' },
        query: { method: 'GET', isArray: true }
    });
}]);
"#;
        let index = analyze_js(source);
        check("$resource factoryのメソッド抽出(update)", has_def(&index, "UserResource.update", SymbolKind::Method));
        check("$resource factoryのメソッド抽出(query)", has_def(&index, "UserResource.query", SymbolKind::Method));
    }

    // --- 3. factory内の var service = {} パターン (service.xxx) ---
    println!("\n--- 3. factory内 var service = {{}} パターン ---");
    {
        let source = r#"
angular.module('app', []).factory('SvcA', [function() {
    var service = {};
    service.doWork = function() {};
    service.name = 'test';
    return service;
}]);
"#;
        let index = analyze_js(source);
        check("var service = {}; service.method パターンのメソッド認識", has_def(&index, "SvcA.doWork", SymbolKind::Method));
        check("var service = {}; service.property パターンのプロパティ認識", has_def(&index, "SvcA.name", SymbolKind::Method));
    }

    // --- 4. factory return オブジェクトリテラル ---
    println!("\n--- 4. factory return オブジェクトリテラル ---");
    {
        let source = r#"
angular.module('app', []).factory('SvcB', [function() {
    return {
        method1: function(a) { return a; },
        method2: function() { return 'hello'; },
        prop1: 'value'
    };
}]);
"#;
        let index = analyze_js(source);
        check("return { method: function } パターンのメソッド認識", has_def(&index, "SvcB.method1", SymbolKind::Method));
        check("return { prop: value } パターンのプロパティ認識", has_def(&index, "SvcB.prop1", SymbolKind::Method));
    }

    // --- 5. ES6 export パターン ---
    println!("\n--- 5. ES6 export パターン ---");
    {
        let source = r#"
export default angular.module('app', []).controller('ExportedCtrl', ['$scope', function($scope) {
    $scope.val = 1;
}]);
"#;
        let index = analyze_js(source);
        check("export default angular.module(...) が認識される", has_def(&index, "ExportedCtrl", SymbolKind::Controller));
    }
    {
        let source = r#"
const app = angular.module('app', []);
export { app };
app.controller('NamedExportCtrl', ['$scope', function($scope) { $scope.x = 1; }]);
"#;
        let index = analyze_js(source);
        check("named export + angular.module() が認識される", has_def(&index, "NamedExportCtrl", SymbolKind::Controller));
    }

    // --- 6. angular.module() を変数に代入 ---
    println!("\n--- 6. angular.module() の変数代入パターン ---");
    {
        let source = r#"
var app = angular.module('app', []);
app.controller('VarModuleCtrl', ['$scope', function($scope) { $scope.x = 1; }]);
app.service('VarModuleSvc', [function() { this.m = function() {}; }]);
"#;
        let index = analyze_js(source);
        check("var app = angular.module(); app.controller() パターン", has_def(&index, "VarModuleCtrl", SymbolKind::Controller));
        check("var app = angular.module(); app.service() パターン", has_def(&index, "VarModuleSvc", SymbolKind::Service));
    }

    // --- 7. $stateProvider (ui-router) パターン ---
    println!("\n--- 7. $stateProvider (ui-router) パターン ---");
    {
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
        check("$stateProvider.state() のcontroller参照(HomeController)", has_ref(&index, "HomeController"));
        check("$stateProvider.state() のcontroller参照(AboutController)", has_ref(&index, "AboutController"));
    }

    // --- 8. $scope.xxx ネストされたプロパティ ---
    println!("\n--- 8. $scope ネストされたプロパティ ---");
    {
        let source = r#"
angular.module('app', []).controller('NestedCtrl', ['$scope', function($scope) {
    $scope.user = {};
    $scope.user.name = 'test';
    $scope.user.email = 'test@example.com';
    $scope.items = [];
    $scope.items[0] = 'first';
}]);
"#;
        let index = analyze_js(source);
        check("$scope.user (オブジェクト代入)", has_def(&index, "NestedCtrl.$scope.user", SymbolKind::ScopeProperty));
        check("$scope.user.name (ネスト代入)", has_def(&index, "NestedCtrl.$scope.user.name", SymbolKind::ScopeProperty));
        check("$scope.items (配列代入)", has_def(&index, "NestedCtrl.$scope.items", SymbolKind::ScopeProperty));
    }

    // --- 9. controller as + this パターン (vm = this) ---
    println!("\n--- 9. controller as (vm = this) メソッド/プロパティ ---");
    {
        let source = r#"
angular.module('app', []).controller('VmCtrl', ['$http', function($http) {
    var vm = this;
    vm.title = 'Hello';
    vm.items = [];
    vm.loadItems = function() {};
    vm.nested = { sub: 'value' };
}]);
"#;
        let index = analyze_js(source);
        check("vm.title がMethod定義として認識される", has_def(&index, "VmCtrl.title", SymbolKind::Method));
        check("vm.items がMethod定義として認識される", has_def(&index, "VmCtrl.items", SymbolKind::Method));
        check("vm.loadItems がMethod定義として認識される", has_def(&index, "VmCtrl.loadItems", SymbolKind::Method));
        check("vm.nested がMethod定義として認識される", has_def(&index, "VmCtrl.nested", SymbolKind::Method));
    }

    // --- 10. self = this パターン ---
    println!("\n--- 10. self = this パターン (vm以外のエイリアス) ---");
    {
        let source = r#"
angular.module('app', []).controller('SelfCtrl', ['$http', function($http) {
    var self = this;
    self.data = [];
    self.load = function() {};
}]);
"#;
        let index = analyze_js(source);
        check("self = this; self.data パターン認識", has_def(&index, "SelfCtrl.data", SymbolKind::Method));
        check("self = this; self.load パターン認識", has_def(&index, "SelfCtrl.load", SymbolKind::Method));
    }

    // --- 11. that = this パターン --- (対応済み)
    println!("\n--- 11. that = this パターン ---");
    {
        let source = r#"
angular.module('app', []).service('ThatSvc', [function() {
    var that = this;
    that.doWork = function() {};
}]);
"#;
        let index = analyze_js(source);
        let result = has_def(&index, "ThatSvc.doWork", SymbolKind::Method);
        check("that = this; that.doWork パターン認識", result);
        assert!(result, "that = this パターンは対応済み");
    }

    // --- 12. component $onInit 等のライフサイクルフック --- (対応済み)
    println!("\n--- 12. component ライフサイクルフック認識 ---");
    {
        let source = r#"
angular.module('app', []).component('lcComp', {
    template: '<div></div>',
    controller: function() {
        var ctrl = this;
        ctrl.data = [];
        ctrl.$onInit = function() {};
        ctrl.$onDestroy = function() {};
        ctrl.$onChanges = function(changes) {};
        ctrl.$doCheck = function() {};
        ctrl.$postLink = function() {};
    }
});
"#;
        let index = analyze_js(source);
        let result_data = has_def(&index, "lcComp.data", SymbolKind::Method);
        let result_init = has_def(&index, "lcComp.$onInit", SymbolKind::Method);
        let result_destroy = has_def(&index, "lcComp.$onDestroy", SymbolKind::Method);
        let result_changes = has_def(&index, "lcComp.$onChanges", SymbolKind::Method);
        check("ctrl.data (コンポーネント内プロパティ)", result_data);
        check("ctrl.$onInit (ライフサイクルフック)", result_init);
        check("ctrl.$onDestroy (ライフサイクルフック)", result_destroy);
        check("ctrl.$onChanges (ライフサイクルフック)", result_changes);
        assert!(result_data, "component controller内の ctrl.data は対応済み");
        assert!(result_init, "component controller内の ctrl.$onInit は対応済み");
        assert!(result_destroy, "component controller内の ctrl.$onDestroy は対応済み");
        assert!(result_changes, "component controller内の ctrl.$onChanges は対応済み");
    }

    // --- 13. provider の $get メソッド ---
    println!("\n--- 13. provider.$get のメソッド抽出 ---");
    {
        let source = r#"
angular.module('app', []).provider('apiProvider', function() {
    this.setUrl = function(url) {};
    this.$get = ['$http', function($http) {
        return {
            request: function(path) { return $http.get(path); },
            post: function(path, data) { return $http.post(path, data); }
        };
    }];
});
"#;
        let index = analyze_js(source);
        check("provider がProviderとして認識される", has_def(&index, "apiProvider", SymbolKind::Provider));
        check("provider.$getのreturnオブジェクトからメソッド抽出", has_def(&index, "apiProvider.request", SymbolKind::Method));
    }

    // --- 14. constant/value のプロパティアクセス ---
    println!("\n--- 14. constant/value のプロパティアクセス ---");
    {
        let source = r#"
angular.module('app', []).constant('CONFIG', {
    API_URL: 'https://api.example.com',
    MAX_RETRIES: 3,
    FEATURES: { dark_mode: true }
});
"#;
        let index = analyze_js(source);
        check("constant オブジェクトのプロパティ(CONFIG.API_URL)認識", has_def(&index, "CONFIG.API_URL", SymbolKind::Method));
    }

    // --- 15. let/const での angular.module 代入 ---
    println!("\n--- 15. let/const での angular.module 代入 ---");
    {
        let source = r#"
const myApp = angular.module('myConstApp', []);
myApp.controller('ConstAppCtrl', ['$scope', function($scope) { $scope.x = 1; }]);
"#;
        let index = analyze_js(source);
        check("const myApp = angular.module(); myApp.controller()", has_def(&index, "ConstAppCtrl", SymbolKind::Controller));
    }
    {
        let source = r#"
let myApp = angular.module('myLetApp', []);
myApp.service('LetAppSvc', [function() { this.x = function() {}; }]);
"#;
        let index = analyze_js(source);
        check("let myApp = angular.module(); myApp.service()", has_def(&index, "LetAppSvc", SymbolKind::Service));
    }

    // --- 16. $scope.$apply / $scope.$digest 内のプロパティ ---
    println!("\n--- 16. $scope.$apply内プロパティ定義 ---");
    {
        let source = r#"
angular.module('app', []).controller('ApplyCtrl', ['$scope', function($scope) {
    $scope.data = null;
    window.callback = function(result) {
        $scope.$apply(function() {
            $scope.data = result;
            $scope.loaded = true;
        });
    };
}]);
"#;
        let index = analyze_js(source);
        check("$scope.data (通常代入)", has_def(&index, "ApplyCtrl.$scope.data", SymbolKind::ScopeProperty));
        check("$scope.loaded ($apply内での新規代入)", has_def(&index, "ApplyCtrl.$scope.loaded", SymbolKind::ScopeProperty));
    }

    // --- 17. Promise .then() 内の $scope 代入 ---
    println!("\n--- 17. Promise .then() 内の $scope 代入 ---");
    {
        let source = r#"
angular.module('app', []).controller('PromiseCtrl', ['$scope', '$http', function($scope, $http) {
    $scope.users = [];
    $http.get('/api/users').then(function(response) {
        $scope.users = response.data;
        $scope.loaded = true;
    });
}]);
"#;
        let index = analyze_js(source);
        check("$scope.users (初期代入)", has_def(&index, "PromiseCtrl.$scope.users", SymbolKind::ScopeProperty));
        // .then()コールバック内の$scopeプロパティは参照として登録されるべき
        check("$scope.loaded (.then内新規代入)", has_def(&index, "PromiseCtrl.$scope.loaded", SymbolKind::ScopeProperty));
    }

    // --- 18. controller内のネスト関数での$scope ---
    println!("\n--- 18. controller内ネスト関数での$scope ---");
    {
        let source = r#"
angular.module('app', []).controller('NestFnCtrl', ['$scope', function($scope) {
    $scope.count = 0;
    function helper() {
        $scope.helperResult = 'from helper';
    }
    $scope.run = function() {
        helper();
    };
}]);
"#;
        let index = analyze_js(source);
        check("ネスト関数内の $scope.helperResult", has_def(&index, "NestFnCtrl.$scope.helperResult", SymbolKind::ScopeProperty));
    }

    // --- 19. angular.extend / angular.merge パターン ---
    println!("\n--- 19. angular.extend / angular.merge パターン ---");
    {
        let source = r#"
angular.module('app', []).controller('ExtendCtrl', ['$scope', function($scope) {
    angular.extend($scope, {
        extProp1: 'hello',
        extProp2: 42,
        extMethod: function() {}
    });
}]);
"#;
        let index = analyze_js(source);
        check("angular.extend($scope, {...}) のプロパティ認識", has_def(&index, "ExtendCtrl.$scope.extProp1", SymbolKind::ScopeProperty));
    }

    // --- 20. controllerAs で class メソッド ---
    println!("\n--- 20. class controller のメソッド (controller as用) ---");
    {
        let source = r#"
class ClassSvc {
    constructor($http) {
        this.http = $http;
    }
    fetchAll() { return this.http.get('/api/all'); }
    fetchOne(id) { return this.http.get('/api/' + id); }
    static create() { return new ClassSvc(); }
}
ClassSvc.$inject = ['$http'];
angular.module('app', []).service('ClassSvc', ClassSvc);
"#;
        let index = analyze_js(source);
        check("class method fetchAll()", has_def(&index, "ClassSvc.fetchAll", SymbolKind::Method));
        check("class method fetchOne(id)", has_def(&index, "ClassSvc.fetchOne", SymbolKind::Method));
        check("static method create() (非対応の可能性)", has_def(&index, "ClassSvc.create", SymbolKind::Method));
    }

    // --- 21. $routeProvider テンプレートバインディング ---
    println!("\n--- 21. $routeProvider テンプレートバインディング ---");
    {
        let source = r#"
angular.module('app', []).config(['$routeProvider', function($routeProvider) {
    $routeProvider
        .when('/home', {
            templateUrl: 'views/home.html',
            controller: 'HomeCtrl'
        });
}]);
"#;
        let index = analyze_js(source);
        let bindings = index.templates.get_all_template_bindings();
        let has_home_binding = bindings
            .iter()
            .any(|b| b.template_path.contains("home.html") && b.controller_name == "HomeCtrl");
        check("$routeProvider テンプレートバインディング", has_home_binding);
    }

    // --- 22. TypeScript風の型注釈コメント ---
    println!("\n--- 22. JSDoc パラメータ付きコメント ---");
    {
        let source = r#"
/**
 * @param {string} id - ユーザーID
 * @param {Object} data - 更新データ
 * @returns {Promise<Object>} 更新結果
 */
angular.module('app', []).service('JsDocSvc', ['$http', function($http) {
    /**
     * データを取得する
     * @param {string} key
     * @returns {Promise}
     */
    this.get = function(key) { return $http.get('/api/' + key); };
}]);
"#;
        let index = analyze_js(source);
        let defs = index.definitions.get_definitions("JsDocSvc");
        let svc_has_docs = defs.iter().any(|d| d.docs.is_some());
        check("サービスレベルのJSDocコメント取得", svc_has_docs);

        let method_defs = index.definitions.get_definitions("JsDocSvc.get");
        let method_has_docs = method_defs.iter().any(|d| d.docs.is_some());
        check("メソッドレベルのJSDocコメント取得", method_has_docs);
    }

    println!("\n========================================\n");
}

// ============================================================
// HTML側の非対応調査
// ============================================================

#[test]
fn investigate_html_unsupported_patterns() {
    println!("\n========================================");
    println!("  AngularJS LSP 非対応構文・機能の調査");
    println!("  (HTML側)");
    println!("========================================\n");

    // --- 1. one-time binding (::) ---
    println!("--- 1. One-time binding (::) ---");
    {
        let js = r#"
angular.module('app', []).controller('OTCtrl', ['$scope', function($scope) {
    $scope.name = 'test';
    $scope.email = 'test@test.com';
}]);
"#;
        let html = r#"
<div ng-controller="OTCtrl">
    <p>{{ ::name }}</p>
    <span ng-bind="::email"></span>
</div>
"#;
        let index = analyze_html(js, html);
        let html_uri = Url::parse("file:///test.html").unwrap();
        let refs = index.html.get_html_scope_references(&html_uri);
        let has_name = refs.iter().any(|r| r.property_path == "name");
        let has_email = refs.iter().any(|r| r.property_path == "email");
        check("{{ ::name }} one-time bindingのスコープ参照", has_name);
        check("ng-bind=\"::email\" one-time bindingのスコープ参照", has_email);
    }

    // --- 2. controller as + property access ---
    println!("\n--- 2. controller as プロパティ参照 ---");
    {
        let js = r#"
angular.module('app', []).controller('CtrlAsTest', [function() {
    this.title = 'Hello';
    this.items = [];
}]);
"#;
        let html = r#"
<div ng-controller="CtrlAsTest as vm">
    <h1>{{ vm.title }}</h1>
    <div ng-repeat="item in vm.items">{{ item.name }}</div>
    <button ng-click="vm.doSomething()">Go</button>
</div>
"#;
        let index = analyze_html(js, html);
        let html_uri = Url::parse("file:///test.html").unwrap();
        let refs = index.html.get_html_scope_references(&html_uri);
        let has_vm_title = refs.iter().any(|r| r.property_path == "vm.title");
        let has_vm_items = refs.iter().any(|r| r.property_path == "vm.items");
        let has_vm_do = refs.iter().any(|r| r.property_path == "vm.doSomething");
        check("{{ vm.title }} controller as参照", has_vm_title);
        check("ng-repeat=\"item in vm.items\" controller as参照", has_vm_items);
        check("ng-click=\"vm.doSomething()\" controller as参照", has_vm_do);
    }

    // --- 3. ng-repeat 特殊変数 ($index, $first, $last等) ---
    println!("\n--- 3. ng-repeat 特殊変数 ---");
    {
        let js = r#"
angular.module('app', []).controller('SpecialVarCtrl', ['$scope', function($scope) {
    $scope.items = [];
}]);
"#;
        let html = r#"
<div ng-controller="SpecialVarCtrl">
    <div ng-repeat="item in items">
        <span>{{ $index }}: {{ item.name }}</span>
        <span ng-show="$first">First!</span>
        <span ng-show="$last">Last!</span>
        <span ng-class="{ 'odd': $odd, 'even': $even }">row</span>
    </div>
</div>
"#;
        let index = analyze_html(js, html);
        let html_uri = Url::parse("file:///test.html").unwrap();
        let refs = index.html.get_html_scope_references(&html_uri);
        let local_vars = index.html.get_all_local_variables(&html_uri);
        let has_index = refs.iter().any(|r| r.property_path == "$index")
            || local_vars.iter().any(|v| v.name == "$index");
        let has_first = refs.iter().any(|r| r.property_path == "$first")
            || local_vars.iter().any(|v| v.name == "$first");
        let has_last = refs.iter().any(|r| r.property_path == "$last")
            || local_vars.iter().any(|v| v.name == "$last");
        let has_odd = refs.iter().any(|r| r.property_path == "$odd")
            || local_vars.iter().any(|v| v.name == "$odd");
        check("ng-repeat $index 特殊変数", has_index);
        check("ng-repeat $first 特殊変数", has_first);
        check("ng-repeat $last 特殊変数", has_last);
        check("ng-repeat $odd 特殊変数", has_odd);
    }

    // --- 4. ng-repeat as (aliasAs) ---
    println!("\n--- 4. ng-repeat as (alias) ---");
    {
        let html = r#"
<div ng-repeat="item in items | filter:query as filtered">
    {{ filtered.length }} items found
</div>
"#;
        let index = analyze_html("", html);
        let html_uri = Url::parse("file:///test.html").unwrap();
        let local_vars = index.html.get_all_local_variables(&html_uri);
        let has_filtered = local_vars.iter().any(|v| v.name == "filtered");
        check("ng-repeat 'as filtered' alias変数", has_filtered);
    }

    // --- 5. filter式のパイプ ---
    println!("\n--- 5. HTML式内のフィルター ---");
    {
        let js = r#"
angular.module('app', []).controller('FilterCtrl', ['$scope', function($scope) {
    $scope.users = [];
    $scope.query = '';
}]);
angular.module('app').filter('capitalize', [function() {
    return function(input) { return input; };
}]);
"#;
        let html = r#"
<div ng-controller="FilterCtrl">
    <p>{{ users | filter:query | orderBy:'name' }}</p>
    <p>{{ user.name | capitalize }}</p>
</div>
"#;
        let index = analyze_html(js, html);
        let html_uri = Url::parse("file:///test.html").unwrap();
        let refs = index.html.get_html_scope_references(&html_uri);
        let has_users = refs.iter().any(|r| r.property_path == "users");
        let has_user_name = refs.iter().any(|r| r.property_path == "user.name");
        check("{{ users | filter:query }} パイプ前の変数参照", has_users);
        check("{{ user.name | capitalize }} パイプ前のネスト参照", has_user_name);
    }

    // --- 6. 三項演算子 ---
    println!("\n--- 6. HTML式内の三項演算子 ---");
    {
        let js = r#"
angular.module('app', []).controller('TernaryCtrl', ['$scope', function($scope) {
    $scope.isActive = true;
    $scope.activeText = 'ON';
    $scope.inactiveText = 'OFF';
}]);
"#;
        let html = r#"
<div ng-controller="TernaryCtrl">
    <p>{{ isActive ? activeText : inactiveText }}</p>
</div>
"#;
        let index = analyze_html(js, html);
        let html_uri = Url::parse("file:///test.html").unwrap();
        let refs = index.html.get_html_scope_references(&html_uri);
        let has_is_active = refs.iter().any(|r| r.property_path == "isActive");
        let has_active_text = refs.iter().any(|r| r.property_path == "activeText");
        let has_inactive_text = refs.iter().any(|r| r.property_path == "inactiveText");
        check("三項演算子の条件部 (isActive)", has_is_active);
        check("三項演算子のtrue部 (activeText)", has_active_text);
        check("三項演算子のfalse部 (inactiveText)", has_inactive_text);
    }

    // --- 7. ng-class のオブジェクト式 ---
    println!("\n--- 7. ng-class オブジェクト式の変数参照 ---");
    {
        let js = r#"
angular.module('app', []).controller('NgClassCtrl', ['$scope', function($scope) {
    $scope.isActive = true;
    $scope.isDisabled = false;
    $scope.dynamicClass = 'my-class';
}]);
"#;
        let html = r#"
<div ng-controller="NgClassCtrl">
    <div ng-class="{ 'active': isActive, 'disabled': isDisabled }">test</div>
    <div ng-class="dynamicClass">test2</div>
</div>
"#;
        let index = analyze_html(js, html);
        let html_uri = Url::parse("file:///test.html").unwrap();
        let refs = index.html.get_html_scope_references(&html_uri);
        let has_active = refs.iter().any(|r| r.property_path == "isActive");
        let has_disabled = refs.iter().any(|r| r.property_path == "isDisabled");
        let has_dynamic = refs.iter().any(|r| r.property_path == "dynamicClass");
        check("ng-class オブジェクト式の変数 (isActive)", has_active);
        check("ng-class オブジェクト式の変数 (isDisabled)", has_disabled);
        check("ng-class 文字列式の変数 (dynamicClass)", has_dynamic);
    }

    // --- 8. ng-options ---
    println!("\n--- 8. ng-options の変数参照 ---");
    {
        let js = r#"
angular.module('app', []).controller('OptionsCtrl', ['$scope', function($scope) {
    $scope.users = [];
    $scope.selectedUser = null;
}]);
"#;
        let html = r#"
<div ng-controller="OptionsCtrl">
    <select ng-model="selectedUser" ng-options="user.name for user in users track by user.id">
    </select>
</div>
"#;
        let index = analyze_html(js, html);
        let html_uri = Url::parse("file:///test.html").unwrap();
        let refs = index.html.get_html_scope_references(&html_uri);
        let has_users = refs.iter().any(|r| r.property_path == "users");
        let has_selected = refs.iter().any(|r| r.property_path == "selectedUser");
        check("ng-options の users 参照", has_users);
        check("ng-model の selectedUser 参照", has_selected);
    }

    // --- 9. component (<hero-detail> bindings) ---
    println!("\n--- 9. component バインディング属性参照 ---");
    {
        let js = r#"
angular.module('app', [])
.component('myWidget', {
    templateUrl: 'widget.html',
    bindings: { data: '<', onUpdate: '&', label: '@' }
})
.controller('ParentCtrl', ['$scope', function($scope) {
    $scope.widgetData = {};
    $scope.handleUpdate = function() {};
}]);
"#;
        let html = r#"
<div ng-controller="ParentCtrl">
    <my-widget data="widgetData" on-update="handleUpdate()" label="My Widget"></my-widget>
</div>
"#;
        let index = analyze_html(js, html);
        let html_uri = Url::parse("file:///test.html").unwrap();
        let refs = index.html.get_html_scope_references(&html_uri);
        let has_widget_data = refs.iter().any(|r| r.property_path == "widgetData");
        let has_handle_update = refs.iter().any(|r| r.property_path == "handleUpdate");
        check("component属性 data=\"widgetData\" の参照", has_widget_data);
        check("component属性 on-update=\"handleUpdate()\" の参照", has_handle_update);
    }

    // --- 10. ng-messages / ng-message ---
    println!("\n--- 10. ng-messages / ng-message ディレクティブ ---");
    {
        let html = r#"
<form name="myForm">
    <input type="text" name="field" ng-model="val" required ng-minlength="3">
    <div ng-messages="myForm.field.$error">
        <div ng-message="required">Required</div>
        <div ng-message="minlength">Too short</div>
    </div>
</form>
"#;
        let index = analyze_html("", html);
        let html_uri = Url::parse("file:///test.html").unwrap();
        let refs = index.html.get_html_scope_references(&html_uri);
        let has_form_ref = refs.iter().any(|r| r.property_path.contains("myForm"));
        check("ng-messages=\"myForm.field.$error\" 参照認識", has_form_ref);
    }

    // --- 11. directive 属性バインディング (isolated scope) ---
    println!("\n--- 11. directive isolated scope バインディング ---");
    {
        let js = r#"
angular.module('app', [])
.directive('myDir', [function() {
    return {
        restrict: 'E',
        scope: {
            title: '@',
            data: '=',
            onChange: '&'
        },
        template: '<div>{{ title }}</div>'
    };
}])
.controller('DirCtrl', ['$scope', function($scope) {
    $scope.myTitle = 'Hello';
    $scope.myData = {};
    $scope.myChange = function() {};
}]);
"#;
        let html = r#"
<div ng-controller="DirCtrl">
    <my-dir title="{{ myTitle }}" data="myData" on-change="myChange()"></my-dir>
</div>
"#;
        let index = analyze_html(js, html);
        let html_uri = Url::parse("file:///test.html").unwrap();
        let refs = index.html.get_html_scope_references(&html_uri);
        let has_my_title = refs.iter().any(|r| r.property_path == "myTitle");
        let has_my_data = refs.iter().any(|r| r.property_path == "myData");
        let has_my_change = refs.iter().any(|r| r.property_path == "myChange");
        check("directive @ バインディング title=\"{{ myTitle }}\"", has_my_title);
        check("directive = バインディング data=\"myData\"", has_my_data);
        check("directive & バインディング on-change=\"myChange()\"", has_my_change);
    }

    // --- 12. ng-repeat ネスト ---
    println!("\n--- 12. ng-repeat ネスト ---");
    {
        let js = r#"
angular.module('app', []).controller('NestedRepeatCtrl', ['$scope', function($scope) {
    $scope.categories = [];
}]);
"#;
        let html = r#"
<div ng-controller="NestedRepeatCtrl">
    <div ng-repeat="category in categories">
        <h3>{{ category.name }}</h3>
        <div ng-repeat="item in category.items">
            <p>{{ item.name }}</p>
        </div>
    </div>
</div>
"#;
        let index = analyze_html(js, html);
        let html_uri = Url::parse("file:///test.html").unwrap();
        let local_vars = index.html.get_all_local_variables(&html_uri);
        let has_category = local_vars.iter().any(|v| v.name == "category");
        let has_item = local_vars.iter().any(|v| v.name == "item");
        check("外側 ng-repeat 変数 (category)", has_category);
        check("内側 ng-repeat 変数 (item)", has_item);
    }

    // --- 13. $event 参照 ---
    println!("\n--- 13. $event 参照 ---");
    {
        let html = r#"
<button ng-click="handleClick($event)">Click</button>
<div ng-mouseover="hover($event)">Hover</div>
"#;
        let index = analyze_html("", html);
        let html_uri = Url::parse("file:///test.html").unwrap();
        let refs = index.html.get_html_scope_references(&html_uri);
        let has_event = refs.iter().any(|r| r.property_path == "$event");
        check("ng-click の $event 参照", has_event);
    }

    // --- 14. カスタム補間シンボル ---
    println!("\n--- 14. カスタム補間シンボル (デフォルト {{ }} のみテスト) ---");
    check("カスタム補間 (ajsconfig.jsonで設定可能、テスト省略)", true);

    println!("\n========================================\n");
}
