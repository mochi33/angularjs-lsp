use std::sync::Arc;
use tower_lsp::lsp_types::Url;

use super::context::AnalyzerContext;
use super::AngularJsAnalyzer;
use crate::analyzer::JsParser;
use crate::index::{SymbolIndex, SymbolKind};

#[test]
fn test_di_check_with_di() {
    // DIされている場合は参照が登録される
    let source = r#"
angular.module('app')
.service('MyService', function() {
    this.doSomething = function() {};
})
.controller('TestCtrl', ['$scope', 'MyService', function($scope, MyService) {
    MyService.doSomething();
}]);
"#;
    let index = Arc::new(SymbolIndex::new());
    let analyzer = AngularJsAnalyzer::new(index.clone());
    let uri = Url::parse("file:///test.js").unwrap();

    // Pass 1: definitions
    analyzer.analyze_document_with_options(&uri, source, true);
    // Pass 2: references
    analyzer.analyze_document_with_options(&uri, source, false);

    // MyService.doSomething への参照が登録されているはず
    let refs = index.get_references("MyService.doSomething");
    assert!(!refs.is_empty(), "DIされている場合は参照が登録されるべき");
}

#[test]
fn test_di_check_without_di() {
    // DIされていない場合は参照が登録されない
    let source = r#"
angular.module('app')
.service('MyService', function() {
    this.doSomething = function() {};
})
.controller('TestCtrl', ['$scope', function($scope) {
    MyService.doSomething();
}]);
"#;
    let index = Arc::new(SymbolIndex::new());
    let analyzer = AngularJsAnalyzer::new(index.clone());
    let uri = Url::parse("file:///test.js").unwrap();

    // Pass 1: definitions
    analyzer.analyze_document_with_options(&uri, source, true);
    // Pass 2: references
    analyzer.analyze_document_with_options(&uri, source, false);

    // MyService.doSomething への参照が登録されていないはず
    let refs = index.get_references("MyService.doSomething");
    assert!(refs.is_empty(), "DIされていない場合は参照が登録されないべき");
}

#[test]
fn test_di_check_inject_pattern() {
    // $inject パターンでDIされている場合は参照が登録される
    let source = r#"
angular.module('app')
.service('MyService', function() {
    this.doSomething = function() {};
});

function TestController($scope, MyService) {
    MyService.doSomething();
}
TestController.$inject = ['$scope', 'MyService'];
"#;
    let index = Arc::new(SymbolIndex::new());
    let analyzer = AngularJsAnalyzer::new(index.clone());
    let uri = Url::parse("file:///test.js").unwrap();

    // Pass 1: definitions
    analyzer.analyze_document_with_options(&uri, source, true);
    // Pass 2: references
    analyzer.analyze_document_with_options(&uri, source, false);

    // MyService.doSomething への参照が登録されているはず
    let refs = index.get_references("MyService.doSomething");
    assert!(!refs.is_empty(), "$injectパターンでDIされている場合は参照が登録されるべき");
}

#[test]
fn test_di_check_inject_pattern_without_di() {
    // $inject パターンでDIされていない場合は参照が登録されない
    let source = r#"
angular.module('app')
.service('MyService', function() {
    this.doSomething = function() {};
});

function TestController($scope) {
    MyService.doSomething();
}
TestController.$inject = ['$scope'];
"#;
    let index = Arc::new(SymbolIndex::new());
    let analyzer = AngularJsAnalyzer::new(index.clone());
    let uri = Url::parse("file:///test.js").unwrap();

    // Pass 1: definitions
    analyzer.analyze_document_with_options(&uri, source, true);
    // Pass 2: references
    analyzer.analyze_document_with_options(&uri, source, false);

    // MyService.doSomething への参照が登録されていないはず
    let refs = index.get_references("MyService.doSomething");
    assert!(refs.is_empty(), "$injectパターンでDIされていない場合は参照が登録されないべき");
}

#[test]
fn test_di_check_iife_inject_pattern() {
    // IIFE内の$injectパターンでDIされている場合は参照が登録される
    let source = r#"
angular.module('app')
.service('notifyService', function() {
    this.showNotify = function() {};
});

(function() {
    'use strict';
    angular
        .module('app')
        .controller('TestController', TestController);

    TestController.$inject = ['notifyService'];

    function TestController(notifyService) {
        notifyService.showNotify();
    }
})();
"#;
    let mut parser = JsParser::new();
    let tree = parser.parse(source).unwrap();
    let mut ctx = AnalyzerContext::new();
    let index = Arc::new(SymbolIndex::new());
    let analyzer = AngularJsAnalyzer::new(index.clone());
    let uri = Url::parse("file:///test.js").unwrap();

    analyzer.collect_function_declarations_for_inject(tree.root_node(), source, &mut ctx);
    analyzer.collect_inject_patterns(tree.root_node(), source, &uri, &mut ctx);
    // Pass 1: definitions
    analyzer.analyze_document_with_options(&uri, source, true);
    // Pass 2: references
    analyzer.analyze_document_with_options(&uri, source, false);

    // notifyService.showNotify への参照が登録されているはず
    let refs = index.get_references("notifyService.showNotify");
    assert!(!refs.is_empty(), "IIFE内の$injectパターンでDIされている場合は参照が登録されるべき: refs={:?}", refs);
}

#[test]
fn test_collect_inject_patterns() {
    // $inject パターンが正しく収集されているか確認
    let source = r#"
(function() {
    TestController.$inject = ['notifyService'];

    function TestController(notifyService) {
        notifyService.showNotify();
    }
})();
"#;
    let mut parser = JsParser::new();
    let tree = parser.parse(source).unwrap();
    let mut ctx = AnalyzerContext::new();

    let index = Arc::new(SymbolIndex::new());
    let analyzer = AngularJsAnalyzer::new(index);
    let uri = Url::parse("file:///test.js").unwrap();

    analyzer.collect_function_declarations_for_inject(tree.root_node(), source, &mut ctx);
    analyzer.collect_inject_patterns(tree.root_node(), source, &uri, &mut ctx);

    assert!(ctx.function_ranges.contains_key("TestController"), "TestController should be in function_ranges");
    assert!(ctx.inject_map.contains_key("TestController"), "TestController should be in inject_map");

    // is_injected_at のテスト
    // 行5 (0-indexed: 4) は関数本体内
    assert!(ctx.is_injected_at("notifyService", 5), "notifyService should be injected at line 5");
    assert!(!ctx.is_injected_at("otherService", 5), "otherService should NOT be injected at line 5");
}

#[test]
fn test_is_injected_at_with_inject_pattern() {
    // is_injected_at が $inject パターンで正しく動作するか確認
    let mut ctx = AnalyzerContext::new();
    ctx.function_ranges.insert("TestController".to_string(), (4, 6));
    ctx.inject_map.insert("TestController".to_string(), vec!["notifyService".to_string()]);

    // 行5は関数本体内 (4 <= 5 <= 6)
    assert!(ctx.is_injected_at("notifyService", 5), "notifyService should be injected at line 5");
    // 行3は関数本体外 (3 < 4)
    assert!(!ctx.is_injected_at("notifyService", 3), "notifyService should NOT be injected at line 3");
    // 存在しないサービス
    assert!(!ctx.is_injected_at("otherService", 5), "otherService should NOT be injected");
}

#[test]
fn test_scope_property_definition() {
    // $scope.xxx = ... が定義として登録される
    let source = r#"
angular.module('app')
.controller('TestCtrl', ['$scope', function($scope) {
    $scope.users = [];
    $scope.loadUsers = function() {
        return [];
    };
}]);
"#;
    let index = Arc::new(SymbolIndex::new());
    let analyzer = AngularJsAnalyzer::new(index.clone());
    let uri = Url::parse("file:///test.js").unwrap();

    // Pass 1: definitions
    analyzer.analyze_document_with_options(&uri, source, true);
    // Pass 2: references
    analyzer.analyze_document_with_options(&uri, source, false);

    // $scope.users の定義が登録されているはず（プロパティ）
    let users_defs = index.get_definitions("TestCtrl.$scope.users");
    assert!(!users_defs.is_empty(), "$scope.users の定義が登録されるべき");
    assert_eq!(users_defs[0].kind, SymbolKind::ScopeProperty);

    // $scope.loadUsers の定義が登録されているはず（メソッド）
    let load_defs = index.get_definitions("TestCtrl.$scope.loadUsers");
    assert!(!load_defs.is_empty(), "$scope.loadUsers の定義が登録されるべき");
    assert_eq!(load_defs[0].kind, SymbolKind::ScopeMethod, "関数は ScopeMethod として登録されるべき");
}

#[test]
fn test_scope_property_reference() {
    // $scope.xxx への参照が登録される
    let source = r#"
angular.module('app')
.controller('TestCtrl', ['$scope', function($scope) {
    $scope.users = [];
    $scope.loadUsers = function() {
        return $scope.users;
    };
}]);
"#;
    let index = Arc::new(SymbolIndex::new());
    let analyzer = AngularJsAnalyzer::new(index.clone());
    let uri = Url::parse("file:///test.js").unwrap();

    // Pass 1: definitions
    analyzer.analyze_document_with_options(&uri, source, true);
    // Pass 2: references
    analyzer.analyze_document_with_options(&uri, source, false);

    // $scope.users への参照が登録されているはず（return $scope.users の部分）
    let refs = index.get_references("TestCtrl.$scope.users");
    assert!(!refs.is_empty(), "$scope.users への参照が登録されるべき");
}

#[test]
fn test_scope_first_definition_only() {
    // 最初の代入のみが定義として登録される
    let source = r#"
angular.module('app')
.controller('TestCtrl', ['$scope', function($scope) {
    $scope.count = 0;
    $scope.count = 1;
    $scope.count = 2;
}]);
"#;
    let index = Arc::new(SymbolIndex::new());
    let analyzer = AngularJsAnalyzer::new(index.clone());
    let uri = Url::parse("file:///test.js").unwrap();

    // Pass 1: definitions
    analyzer.analyze_document_with_options(&uri, source, true);

    // 定義は1つだけ
    let defs = index.get_definitions("TestCtrl.$scope.count");
    assert_eq!(defs.len(), 1, "最初の定義のみが登録されるべき");
    // 最初の定義は行3（0-indexed）
    assert_eq!(defs[0].start_line, 3, "最初の定義の行が正しくない");
}

#[test]
fn test_scope_inject_pattern() {
    // $inject パターンでの $scope プロパティ
    let source = r#"
angular.module('app')
.controller('TestCtrl', TestCtrl);

TestCtrl.$inject = ['$scope'];

function TestCtrl($scope) {
    $scope.message = 'Hello';
}
"#;
    let index = Arc::new(SymbolIndex::new());
    let analyzer = AngularJsAnalyzer::new(index.clone());
    let uri = Url::parse("file:///test.js").unwrap();

    // Pass 1: definitions
    analyzer.analyze_document_with_options(&uri, source, true);

    // $scope.message の定義が登録されているはず
    let defs = index.get_definitions("TestCtrl.$scope.message");
    assert!(!defs.is_empty(), "$inject パターンでも $scope.message の定義が登録されるべき");
}

#[test]
fn test_scope_without_di() {
    // $scope がDIされていない場合は定義が登録されない
    let source = r#"
angular.module('app')
.controller('TestCtrl', ['$http', function($http) {
    $scope.users = [];
}]);
"#;
    let index = Arc::new(SymbolIndex::new());
    let analyzer = AngularJsAnalyzer::new(index.clone());
    let uri = Url::parse("file:///test.js").unwrap();

    // Pass 1: definitions
    analyzer.analyze_document_with_options(&uri, source, true);

    // $scope がDIされていないので、定義は登録されないはず
    let defs = index.get_definitions("TestCtrl.$scope.users");
    assert!(defs.is_empty(), "$scope がDIされていない場合は定義が登録されないべき");
}

#[test]
fn test_scope_reference_without_definition() {
    // 定義がなくても参照が登録される（非同期処理内での定義など）
    let source = r#"
angular.module('app')
.controller('TestCtrl', ['$scope', '$http', function($scope, $http) {
    $http.get('/api/data').then(function(response) {
        $scope.asyncData = response.data;
    });

    // asyncData を参照（定義は非同期処理内）
    console.log($scope.asyncData);
}]);
"#;
    let index = Arc::new(SymbolIndex::new());
    let analyzer = AngularJsAnalyzer::new(index.clone());
    let uri = Url::parse("file:///test.js").unwrap();

    // Pass 1: definitions
    analyzer.analyze_document_with_options(&uri, source, true);
    // Pass 2: references
    analyzer.analyze_document_with_options(&uri, source, false);

    // 定義がなくても参照が登録されているはず
    let refs = index.get_references("TestCtrl.$scope.asyncData");
    assert!(!refs.is_empty(), "定義がなくても参照が登録されるべき");
    // 2箇所の参照（代入の右辺とconsole.log内）
    assert_eq!(refs.len(), 1, "console.log内の参照が登録されるべき（代入は定義として扱われる）");
}

#[test]
fn test_scope_find_all_references_without_definition() {
    // 定義がなくても参照同士を検索できる
    let source = r#"
angular.module('app')
.controller('TestCtrl', ['$scope', '$http', function($scope, $http) {
    $http.get('/api').then(function(res) {
        $scope.items = res.data;
    });

    $scope.items.forEach(function(item) {});
    console.log($scope.items);
}]);
"#;
    let index = Arc::new(SymbolIndex::new());
    let analyzer = AngularJsAnalyzer::new(index.clone());
    let uri = Url::parse("file:///test.js").unwrap();

    // Pass 1: definitions
    analyzer.analyze_document_with_options(&uri, source, true);
    // Pass 2: references
    analyzer.analyze_document_with_options(&uri, source, false);

    // 参照が複数登録されているはず
    let refs = index.get_references("TestCtrl.$scope.items");
    assert!(refs.len() >= 2, "複数の参照が登録されるべき: {:?}", refs);

    // 参照位置からシンボル名を取得できる
    let symbol_name = index.find_symbol_at_position(&uri, refs[0].start_line, refs[0].start_col);
    assert_eq!(symbol_name, Some("TestCtrl.$scope.items".to_string()), "参照位置からシンボル名を取得できるべき");
}

#[test]
fn test_scope_in_nested_function() {
    // ネストされた関数内での $scope 参照
    let source = r#"
angular.module('app')
.controller('TestCtrl', ['$scope', function($scope) {
    $scope.count = 0;

    function init() {
        $scope.count = 10;
        $scope.message = 'Hello';
    }

    function helper() {
        return $scope.count + 1;
    }

    init();
}]);
"#;
    let index = Arc::new(SymbolIndex::new());
    let analyzer = AngularJsAnalyzer::new(index.clone());
    let uri = Url::parse("file:///test.js").unwrap();

    // Pass 1: definitions
    analyzer.analyze_document_with_options(&uri, source, true);
    // Pass 2: references
    analyzer.analyze_document_with_options(&uri, source, false);

    // ネストされた関数内での定義も登録されるはず
    let message_defs = index.get_definitions("TestCtrl.$scope.message");
    assert!(!message_defs.is_empty(), "$scope.message の定義が登録されるべき: {:?}", message_defs);

    // ネストされた関数内での参照も登録されるはず
    let count_refs = index.get_references("TestCtrl.$scope.count");
    // helper内のreturn $scope.count + 1 (参照)
    assert!(count_refs.len() >= 1, "helper内の$scope.count参照が登録されるべき: count={}, refs={:?}", count_refs.len(), count_refs);
}

#[test]
fn test_scope_in_callback() {
    // コールバック関数内での $scope 参照
    let source = r#"
angular.module('app')
.controller('TestCtrl', ['$scope', '$http', function($scope, $http) {
    $scope.users = [];

    $http.get('/api/users').then(function(response) {
        $scope.users = response.data;
        $scope.loaded = true;
    });

    $scope.refresh = function() {
        $http.get('/api/users').then(function(res) {
            $scope.users = res.data;
        });
    };
}]);
"#;
    let index = Arc::new(SymbolIndex::new());
    let analyzer = AngularJsAnalyzer::new(index.clone());
    let uri = Url::parse("file:///test.js").unwrap();

    // Pass 1: definitions
    analyzer.analyze_document_with_options(&uri, source, true);
    // Pass 2: references
    analyzer.analyze_document_with_options(&uri, source, false);

    // コールバック関数内での定義も登録されるはず
    let loaded_defs = index.get_definitions("TestCtrl.$scope.loaded");
    assert!(!loaded_defs.is_empty(), "コールバック内の$scope.loaded の定義が登録されるべき: {:?}", loaded_defs);

    // コールバック関数内での参照も登録されるはず
    let users_refs = index.get_references("TestCtrl.$scope.users");
    // .then 内の2箇所の$scope.users
    assert!(users_refs.len() >= 2, "コールバック内の$scope.users参照が登録されるべき: count={}, refs={:?}", users_refs.len(), users_refs);
}

#[test]
fn test_scope_in_deeply_nested_callback() {
    // 深くネストされたコールバック内での $scope 参照が同一シンボルとして扱われる
    let source = r#"
angular.module('app')
.controller('DeepCtrl', ['$scope', '$http', '$timeout', function($scope, $http, $timeout) {
    $scope.data = null;

    $http.get('/api/data').then(function(response) {
        $timeout(function() {
            Promise.resolve().then(function() {
                $scope.data = response.data;
                console.log($scope.data);
            });
        }, 100);
    });
}]);
"#;
    let index = Arc::new(SymbolIndex::new());
    let analyzer = AngularJsAnalyzer::new(index.clone());
    let uri = Url::parse("file:///test.js").unwrap();

    // Pass 1: definitions
    analyzer.analyze_document_with_options(&uri, source, true);
    // Pass 2: references
    analyzer.analyze_document_with_options(&uri, source, false);

    // 全ての $scope.data が DeepCtrl.$scope.data として認識されるべき
    let data_defs = index.get_definitions("DeepCtrl.$scope.data");
    assert!(!data_defs.is_empty(), "深くネストされたコールバック内でも$scope.data の定義が登録されるべき");

    // 参照も同じシンボル名で登録される
    let data_refs = index.get_references("DeepCtrl.$scope.data");
    // console.log($scope.data) の参照
    assert!(!data_refs.is_empty(), "深くネストされたコールバック内でも$scope.data の参照が登録されるべき");

    // 定義と参照が全て同じシンボル名を使用している（UnknownController ではない）
    let unknown_defs = index.get_definitions("UnknownController.$scope.data");
    assert!(unknown_defs.is_empty(), "UnknownController.$scope.data が存在すべきではない");

    let unknown_refs = index.get_references("UnknownController.$scope.data");
    assert!(unknown_refs.is_empty(), "UnknownController.$scope.data の参照が存在すべきではない");
}

#[test]
fn test_scope_consistency_between_definition_and_reference() {
    // 定義と参照が同じコントローラー名を使用することを確認
    let source = r#"
angular.module('app')
.controller('ConsistentCtrl', ['$scope', function($scope) {
    $scope.counter = 0;

    function increment() {
        $scope.counter = $scope.counter + 1;
    }

    $scope.increment = function() {
        increment();
        return $scope.counter;
    };
}]);
"#;
    let index = Arc::new(SymbolIndex::new());
    let analyzer = AngularJsAnalyzer::new(index.clone());
    let uri = Url::parse("file:///test.js").unwrap();

    // Pass 1: definitions
    analyzer.analyze_document_with_options(&uri, source, true);
    // Pass 2: references
    analyzer.analyze_document_with_options(&uri, source, false);

    // $scope.counter の定義
    let counter_defs = index.get_definitions("ConsistentCtrl.$scope.counter");
    assert_eq!(counter_defs.len(), 1, "$scope.counter の定義は1つのみ");

    // $scope.counter への参照（increment関数内の2箇所 + incrementメソッド内の1箇所）
    let counter_refs = index.get_references("ConsistentCtrl.$scope.counter");
    assert!(counter_refs.len() >= 2, "$scope.counter への参照が複数あるべき: count={}, refs={:?}", counter_refs.len(), counter_refs);

    // $scope.increment の定義
    let inc_defs = index.get_definitions("ConsistentCtrl.$scope.increment");
    assert_eq!(inc_defs.len(), 1, "$scope.increment の定義は1つのみ");
}
