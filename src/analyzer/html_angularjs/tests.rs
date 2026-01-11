use super::*;
use crate::index::SymbolIndex;
use rstest::{fixture, rstest};

/// テスト用のアナライザーとインデックスのペア
struct TestContext {
    analyzer: HtmlAngularJsAnalyzer,
    js_analyzer: Arc<AngularJsAnalyzer>,
    index: Arc<SymbolIndex>,
}

impl TestContext {
    /// HTMLドキュメントを解析（<script>タグ内のJSも解析）
    fn analyze_document(&self, uri: &Url, html: &str) {
        // まず<script>タグ内のJSを解析
        let scripts = HtmlAngularJsAnalyzer::extract_scripts(html);
        for script in scripts {
            self.js_analyzer.analyze_embedded_script(uri, &script.source, script.line_offset);
        }
        // 次にHTML解析
        self.analyzer.analyze_document(uri, html);
    }
}

#[fixture]
fn ctx() -> TestContext {
    let index = Arc::new(SymbolIndex::new());
    let js_analyzer = Arc::new(AngularJsAnalyzer::new(Arc::clone(&index)));
    let analyzer = HtmlAngularJsAnalyzer::new(Arc::clone(&index), Arc::clone(&js_analyzer));
    TestContext { analyzer, js_analyzer, index }
}

#[rstest]
fn test_ng_controller_scope_detection(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="UserController">
    <span>Hello</span>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // ng-controllerスコープが検出されているか
    let controller = index.get_html_controller_at(&uri, 1);
    assert_eq!(controller, Some("UserController".to_string()));
}

#[rstest]
fn test_nested_ng_controller(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="OuterController">
    <div ng-controller="InnerController">
        <span>Inner</span>
    </div>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // 外側のコントローラー
    let outer = index.get_html_controller_at(&uri, 1);
    assert_eq!(outer, Some("OuterController".to_string()));

    // 内側のコントローラー（より狭いスコープを優先）
    let inner = index.get_html_controller_at(&uri, 3);
    assert_eq!(inner, Some("InnerController".to_string()));
}

#[rstest]
fn test_ng_model_reference(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="UserController">
    <input ng-model="user.name">
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // ng-modelからスコープ参照が抽出されているか
    // "user" starts at column 21 in `    <input ng-model="user.name">`
    let ref_opt = index.find_html_scope_reference_at(&uri, 2, 21);
    assert!(ref_opt.is_some(), "ng-model reference should be found");
    assert_eq!(ref_opt.unwrap().property_path, "user");
}

/// 自己終了タグ（<input ... />）からのスコープ参照テスト
#[rstest]
fn test_self_closing_input_reference(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    // 自己終了形式（/>で終わる）
    let html = r#"
<div ng-controller="UserController">
    <input ng-model="user.name" ng-class="{'error': isError()}" />
</div>
"#;

    analyzer.analyze_document(&uri, html);

    // ng-modelからスコープ参照が抽出されているか
    let refs = index.html_scope_references_for_test(&uri).unwrap_or_default();
    eprintln!("All refs: {:?}", refs.iter().map(|r| (&r.property_path, r.start_line, r.start_col)).collect::<Vec<_>>());

    assert!(refs.iter().any(|r| r.property_path == "user"),
        "ng-model reference in self-closing input should be found, got: {:?}",
        refs.iter().map(|r| &r.property_path).collect::<Vec<_>>());
    assert!(refs.iter().any(|r| r.property_path == "isError"),
        "isError reference in self-closing input should be found, got: {:?}",
        refs.iter().map(|r| &r.property_path).collect::<Vec<_>>());
}

#[rstest]
fn test_ng_click_reference(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="UserController">
    <button ng-click="save()">Save</button>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // ng-clickからスコープ参照が抽出されているか
    let ref_opt = index.find_html_scope_reference_at(&uri, 2, 22);
    assert!(ref_opt.is_some(), "ng-click reference should be found");
    assert_eq!(ref_opt.unwrap().property_path, "save");
}

#[rstest]
fn test_ng_repeat_reference(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="UserController">
    <div ng-repeat="item in items">{{item.name}}</div>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // ng-repeatからコレクション参照が抽出されているか
    // "items" starts at column 28 in `    <div ng-repeat="item in items">`
    let ref_opt = index.find_html_scope_reference_at(&uri, 2, 28);
    assert!(ref_opt.is_some(), "ng-repeat reference should be found");
    assert_eq!(ref_opt.unwrap().property_path, "items");
}

#[rstest]
fn test_interpolation_reference(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="UserController">
    <span>{{message}}</span>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // {{interpolation}}からスコープ参照が抽出されているか
    let ref_opt = index.find_html_scope_reference_at(&uri, 2, 12);
    assert!(ref_opt.is_some(), "interpolation reference should be found");
    assert_eq!(ref_opt.unwrap().property_path, "message");
}

#[rstest]
fn test_ng_if_reference(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="UserController">
    <span ng-if="isVisible">Hello</span>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    let ref_opt = index.find_html_scope_reference_at(&uri, 2, 18);
    assert!(ref_opt.is_some(), "ng-if reference should be found");
    assert_eq!(ref_opt.unwrap().property_path, "isVisible");
}

#[rstest]
fn test_ng_show_reference(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="UserController">
    <span ng-show="showMessage">Hello</span>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    let ref_opt = index.find_html_scope_reference_at(&uri, 2, 20);
    assert!(ref_opt.is_some(), "ng-show reference should be found");
    assert_eq!(ref_opt.unwrap().property_path, "showMessage");
}

#[rstest]
fn test_script_tag_route_binding(ctx: TestContext) {
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<script>
angular.module('app').config(function($routeProvider) {
    $routeProvider.when('/users', {
        controller: 'UserController',
        templateUrl: 'views/users.html'
    });
});
</script>
"#;
    ctx.analyze_document(&uri, html);

    // テンプレートバインディングが抽出されているか
    let controller = ctx.index.get_controller_for_template(
        &Url::parse("file:///views/users.html").unwrap()
    );
    assert_eq!(controller, Some("UserController".to_string()));
}

#[rstest]
fn test_script_tag_modal_binding(ctx: TestContext) {
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<script>
$uibModal.open({
    controller: 'ModalController',
    templateUrl: 'views/modal.html'
});
</script>
"#;
    ctx.analyze_document(&uri, html);

    // モーダルバインディングが抽出されているか
    let controller = ctx.index.get_controller_for_template(
        &Url::parse("file:///views/modal.html").unwrap()
    );
    assert_eq!(controller, Some("ModalController".to_string()));
}

#[rstest]
fn test_data_ng_controller(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div data-ng-controller="UserController">
    <span>Hello</span>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // data-ng-controllerも認識されるか
    let controller = index.get_html_controller_at(&uri, 1);
    assert_eq!(controller, Some("UserController".to_string()));
}

#[rstest]
fn test_complex_expression(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="UserController">
    <span ng-if="isActive && isEnabled">Active</span>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // 複数のプロパティが抽出されているか（最初の一つをテスト）
    let ref_opt = index.find_html_scope_reference_at(&uri, 2, 18);
    assert!(ref_opt.is_some(), "complex expression reference should be found");
}

#[rstest]
fn test_filter_expression(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="UserController">
    <span>{{amount | currency}}</span>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // フィルター付き式からプロパティが抽出されているか
    let ref_opt = index.find_html_scope_reference_at(&uri, 2, 13);
    assert!(ref_opt.is_some(), "filter expression reference should be found");
    assert_eq!(ref_opt.unwrap().property_path, "amount");
}

#[rstest]
fn test_resolve_controller_for_html(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="UserController">
    <input ng-model="userName">
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // resolve_controller_for_htmlが正しく動作するか
    let controller = index.resolve_controller_for_html(&uri, 2);
    assert_eq!(controller, Some("UserController".to_string()));
}

#[rstest]
fn test_template_binding_resolution(ctx: TestContext) {
    let uri = Url::parse("file:///app.html").unwrap();

    let html = r#"
<script>
$routeProvider.when('/profile', {
    controller: 'ProfileController',
    templateUrl: 'views/profile.html?v=123'
});
</script>
"#;
    ctx.analyze_document(&uri, html);

    // クエリパラメータ付きテンプレートパスが正しく解決されるか
    let template_uri = Url::parse("file:///views/profile.html").unwrap();
    let controller = ctx.index.resolve_controller_for_html(&template_uri, 0);
    assert_eq!(controller, Some("ProfileController".to_string()));
}

#[rstest]
fn test_method_call_with_arguments(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="UserController">
    <button ng-click="vm.save(user.id)">Save</button>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // vm.save(user.id) から vm と user の両方が抽出されているか
    let symbols = index.get_document_symbols(&uri);
    let scope_props: Vec<_> = symbols.iter().filter(|s| s.kind == crate::index::SymbolKind::ScopeProperty).collect();

    let names: Vec<_> = scope_props.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"vm"), "vm should be extracted from ng-click");
    assert!(names.contains(&"user"), "user should be extracted from ng-click arguments");
}

#[rstest]
fn test_ng_repeat_key_value(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="UserController">
    <div ng-repeat="(key, value) in items">{{key}}: {{value}}</div>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // (key, value) in items から items のみが抽出され、key/value は除外されているか
    let symbols = index.get_document_symbols(&uri);
    let scope_props: Vec<_> = symbols.iter().filter(|s| s.kind == crate::index::SymbolKind::ScopeProperty).collect();

    let names: Vec<_> = scope_props.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"items"), "items should be extracted from ng-repeat");
    // key, valueはローカル変数なので除外
    assert!(!names.iter().any(|n| *n == "key"), "key should NOT be extracted (local var)");
    assert!(!names.iter().any(|n| *n == "value"), "value should NOT be extracted (local var)");
}

#[rstest]
fn test_nested_member_expression(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="UserController">
    <span ng-if="vm.user.isActive">Active</span>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // vm.user.isActive から vm のみが抽出されているか
    let symbols = index.get_document_symbols(&uri);
    let scope_props: Vec<_> = symbols.iter().filter(|s| s.kind == crate::index::SymbolKind::ScopeProperty).collect();

    let names: Vec<_> = scope_props.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"vm"), "vm should be extracted from nested member expression");
}

#[rstest]
fn test_ternary_expression(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="UserController">
    <span>{{isActive ? activeLabel : inactiveLabel}}</span>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // 三項演算子から全ての識別子が抽出されているか
    let symbols = index.get_document_symbols(&uri);
    let scope_props: Vec<_> = symbols.iter().filter(|s| s.kind == crate::index::SymbolKind::ScopeProperty).collect();

    let names: Vec<_> = scope_props.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"isActive"), "isActive should be extracted");
    assert!(names.contains(&"activeLabel"), "activeLabel should be extracted");
    assert!(names.contains(&"inactiveLabel"), "inactiveLabel should be extracted");
}

#[rstest]
fn test_custom_interpolate_symbols(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    // カスタムinterpolate記号を設定
    analyzer.set_interpolate_config(crate::config::InterpolateConfig {
        start_symbol: "[[".to_string(),
        end_symbol: "]]".to_string(),
    });

    let html = r#"
<div ng-controller="UserController">
    <span>[[message]]</span>
    <span>{{ignored}}</span>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // [[ ... ]] からは抽出されるが、{{ ... }} からは抽出されない
    let symbols = index.get_document_symbols(&uri);
    let scope_props: Vec<_> = symbols.iter().filter(|s| s.kind == crate::index::SymbolKind::ScopeProperty).collect();

    let names: Vec<_> = scope_props.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"message"), "message should be extracted from [[...]]");
    assert!(!names.contains(&"ignored"), "ignored should NOT be extracted from {{...}}");
}

#[rstest]
fn test_custom_interpolate_with_expressions(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    // カスタムinterpolate記号を設定（ERBスタイルは避け、より一般的な記号を使用）
    analyzer.set_interpolate_config(crate::config::InterpolateConfig {
        start_symbol: "[[".to_string(),
        end_symbol: "]]".to_string(),
    });

    let html = r#"
<div ng-controller="UserController">
    <span>[[ user.name ]]</span>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    let symbols = index.get_document_symbols(&uri);
    let scope_props: Vec<_> = symbols.iter().filter(|s| s.kind == crate::index::SymbolKind::ScopeProperty).collect();

    let names: Vec<_> = scope_props.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"user"), "user should be extracted from [[...]]");
}

#[rstest]
fn test_inline_interpolation_position(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    // カスタムinterpolate記号を設定
    analyzer.set_interpolate_config(crate::config::InterpolateConfig {
        start_symbol: "[[".to_string(),
        end_symbol: "]]".to_string(),
    });

    // インラインで複数要素がある場合のテスト
    // <label>今日の登録数：<span class="text-red">[[ today_client_total_cnt ]]</span>&nbsp;件</label>
    let html = r#"<div ng-controller="TestController"><label>count:<span class="text-red">[[ total_cnt ]]</span>items</label></div>"#;
    analyzer.analyze_document(&uri, html);

    // 位置を確認: <label>count:<span class="text-red">[[ total_cnt ]]</span>items</label>
    //                                                   ^ total_cnt starts here
    // 全体の構造: <div ng-controller="TestController"><label>count:<span class="text-red">[[ total_cnt ]]</span>items</label></div>
    // col 0: <
    // col 36: <label>
    // col 43: count:
    // col 49: <span class="text-red">
    // col 71: [[ total_cnt ]]
    // col 74: total_cnt (after "[[ ")

    // まず登録された参照を全て取得して位置を確認
    let refs = index.html_scope_references_for_test(&uri).unwrap_or_default();
    eprintln!("All HTML scope references:");
    for r in &refs {
        eprintln!("  {} at {}:{}-{}:{}", r.property_path, r.start_line, r.start_col, r.end_line, r.end_col);
    }

    // total_cntの位置を確認 (col 75が正しい位置)
    let ref_opt = index.find_html_scope_reference_at(&uri, 0, 75);
    assert!(ref_opt.is_some(), "inline interpolation reference should be found at col 75");
    assert_eq!(ref_opt.unwrap().property_path, "total_cnt");
}

#[rstest]
fn test_inline_interpolation_with_japanese(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    // カスタムinterpolate記号を設定
    analyzer.set_interpolate_config(crate::config::InterpolateConfig {
        start_symbol: "[[".to_string(),
        end_symbol: "]]".to_string(),
    });

    // ユーザーの実際のケース: 日本語テキストを含むインライン要素
    let html = r#"<div ng-controller="TestController"><label>今日の登録数：<span class="text-red">[[ today_client_total_cnt ]]</span>&nbsp;件</label></div>"#;
    analyzer.analyze_document(&uri, html);

    // まず登録された参照を全て取得して位置を確認
    let refs = index.html_scope_references_for_test(&uri).unwrap_or_default();
    eprintln!("All HTML scope references (Japanese case):");
    for r in &refs {
        eprintln!("  {} at {}:{}-{}:{}", r.property_path, r.start_line, r.start_col, r.end_line, r.end_col);
    }

    // HTMLの位置を手動で計算（UTF-8バイト単位ではなく、tree-sitterは通常バイトオフセットを使用）
    // tree-sitterはバイトオフセットを使用するので、日本語文字は3バイト
    // <div ng-controller="TestController"><label>今日の登録数：<span class="text-red">[[ today_client_total_cnt ]]</span>&nbsp;件</label></div>
    // "今日の登録数：" = 7文字 × 3バイト = 21バイト

    // 参照が正しく登録されているか確認
    assert!(!refs.is_empty(), "Japanese inline interpolation should register references");
    assert_eq!(refs[0].property_path, "today_client_total_cnt");
}

#[rstest]
fn test_ng_if_multiple_function_calls(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    // ng-if内に複数の関数呼び出しがある場合のテスト
    // || 演算子がAngularJSフィルターの | と間違えられないことを確認
    let html = r#"<div ng-controller="TestController"><div ng-if="(isExpense() || isChangeDeadline()) && RequestService.isVisibleRelatedRequest(form_data.related_request_view_setting_flg)">Content</div></div>"#;
    analyzer.analyze_document(&uri, html);

    let refs = index.html_scope_references_for_test(&uri).unwrap_or_default();

    // isExpense が登録されているか
    let is_expense = refs.iter().find(|r| r.property_path == "isExpense");
    assert!(is_expense.is_some(), "isExpense should be registered");

    // isChangeDeadline が登録されているか（|| の後）
    let is_change_deadline = refs.iter().find(|r| r.property_path == "isChangeDeadline");
    assert!(is_change_deadline.is_some(), "isChangeDeadline should be registered");

    // isChangeDeadline の位置でfind_html_scope_reference_atが動作するか確認
    let is_change_deadline = is_change_deadline.unwrap();
    let found = index.find_html_scope_reference_at(&uri, is_change_deadline.start_line, is_change_deadline.start_col);
    assert!(found.is_some(), "Should find isChangeDeadline at its registered position");
}

#[rstest]
fn test_html_scope_reference_registered_as_symbol_reference(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="UserController">
    <input ng-model="userName">
    <span>{{userMessage}}</span>
    <button ng-click="save()">Save</button>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // get_all_references でHTML参照が解決されるか確認
    let user_name_refs = index.get_all_references("UserController.$scope.userName");
    assert!(!user_name_refs.is_empty(), "userName should be found via get_all_references");

    let user_message_refs = index.get_all_references("UserController.$scope.userMessage");
    assert!(!user_message_refs.is_empty(), "userMessage should be found via get_all_references");

    let save_refs = index.get_all_references("UserController.$scope.save");
    assert!(!save_refs.is_empty(), "save should be found via get_all_references");
}

#[rstest]
fn test_html_scope_reference_with_template_binding(ctx: TestContext) {
    let app_uri = Url::parse("file:///app.html").unwrap();

    // まずテンプレートバインディングを設定
    let app_html = r#"
<script>
$routeProvider.when('/users', {
    controller: 'UserController',
    templateUrl: 'views/users.html'
});
</script>
"#;
    ctx.analyze_document(&app_uri, app_html);

    // テンプレートファイルを解析
    let template_uri = Url::parse("file:///views/users.html").unwrap();
    let template_html = r#"
<div>
    <span>{{userName}}</span>
</div>
"#;
    ctx.analyze_document(&template_uri, template_html);

    // テンプレートバインディング経由でコントローラー名が解決されるか
    let refs = ctx.index.get_all_references("UserController.$scope.userName");
    assert!(!refs.is_empty(), "userName should be found via template binding");
    assert_eq!(refs[0].uri, template_uri);
}

#[rstest]
fn test_html_scope_reference_in_ng_if(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="AppController">
    <span ng-if="isVisible && isEnabled">Content</span>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // ng-if内の複数の識別子がget_all_referencesで見つかるか
    let is_visible_refs = index.get_all_references("AppController.$scope.isVisible");
    assert!(!is_visible_refs.is_empty(), "isVisible should be found via get_all_references");

    let is_enabled_refs = index.get_all_references("AppController.$scope.isEnabled");
    assert!(!is_enabled_refs.is_empty(), "isEnabled should be found via get_all_references");
}

#[rstest]
fn test_is_in_angular_context_ng_if(ctx: TestContext) {
    let TestContext { analyzer, .. } = ctx;

    let html = r#"<div ng-controller="UserController">
    <span ng-if="isVisible">Content</span>
</div>"#;

    // ng-if="isVisible" 内にカーソルがある場合
    // 行1、列17 は ng-if=" の直後
    assert!(analyzer.is_in_angular_context(html, 1, 17), "Should be in Angular context (ng-if start)");

    // 行1、列25 は isVisible の途中
    assert!(analyzer.is_in_angular_context(html, 1, 22), "Should be in Angular context (ng-if middle)");

    // 行0、列5 は ng-controller 属性外
    assert!(!analyzer.is_in_angular_context(html, 0, 5), "Should NOT be in Angular context (outside)");
}

#[rstest]
fn test_is_in_angular_context_interpolation(ctx: TestContext) {
    let TestContext { analyzer, .. } = ctx;

    let html = r#"<div ng-controller="UserController">
    <span>{{message}}</span>
</div>"#;

    // {{ の直後
    assert!(analyzer.is_in_angular_context(html, 1, 12), "Should be in Angular context (interpolation start)");

    // message の途中
    assert!(analyzer.is_in_angular_context(html, 1, 15), "Should be in Angular context (interpolation middle)");

    // }} の外
    assert!(!analyzer.is_in_angular_context(html, 1, 5), "Should NOT be in Angular context (outside interpolation)");
}

#[rstest]
fn test_is_in_angular_context_ng_model(ctx: TestContext) {
    let TestContext { analyzer, .. } = ctx;

    let html = r#"<input ng-model="userName">"#;

    // ng-model=" の直後
    assert!(analyzer.is_in_angular_context(html, 0, 17), "Should be in Angular context (ng-model)");

    // userName の途中
    assert!(analyzer.is_in_angular_context(html, 0, 20), "Should be in Angular context (ng-model middle)");
}

#[rstest]
fn test_is_in_angular_context_ng_click(ctx: TestContext) {
    let TestContext { analyzer, .. } = ctx;

    let html = r#"<button ng-click="save()">Save</button>"#;

    // ng-click=" の直後
    assert!(analyzer.is_in_angular_context(html, 0, 18), "Should be in Angular context (ng-click)");
}

#[rstest]
fn test_is_in_angular_context_custom_interpolate(ctx: TestContext) {
    let TestContext { analyzer, .. } = ctx;

    // カスタムinterpolate記号を設定
    analyzer.set_interpolate_config(crate::config::InterpolateConfig {
        start_symbol: "[[".to_string(),
        end_symbol: "]]".to_string(),
    });

    let html = r#"<span>[[message]]</span>"#;

    // [[ の直後
    assert!(analyzer.is_in_angular_context(html, 0, 8), "Should be in Angular context (custom interpolation)");

    // デフォルトの {{ は認識されない
    let html_default = r#"<span>{{message}}</span>"#;
    assert!(!analyzer.is_in_angular_context(html_default, 0, 8), "Should NOT be in Angular context (wrong symbols)");
}

#[rstest]
fn test_ng_include_attribute_detection(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///parent.html").unwrap();

    let html = r#"
<div ng-controller="ParentController">
    <div ng-controller="ChildController">
        <div ng-include="'child.html'"></div>
    </div>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // ng-includeで継承されるコントローラーを確認
    let child_uri = Url::parse("file:///child.html").unwrap();
    let inherited = index.get_inherited_controllers_for_template(&child_uri);
    assert_eq!(inherited.len(), 2, "Should inherit 2 controllers");
    assert_eq!(inherited[0], "ParentController");
    assert_eq!(inherited[1], "ChildController");
}

#[rstest]
fn test_ng_include_element_detection(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///parent.html").unwrap();

    let html = r#"
<div ng-controller="MainController">
    <ng-include src="'partial.html'"></ng-include>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // ng-include要素で継承されるコントローラーを確認
    let child_uri = Url::parse("file:///partial.html").unwrap();
    let inherited = index.get_inherited_controllers_for_template(&child_uri);
    assert_eq!(inherited.len(), 1, "Should inherit 1 controller");
    assert_eq!(inherited[0], "MainController");
}

#[rstest]
fn test_child_html_multiple_controller_references(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let parent_uri = Url::parse("file:///parent.html").unwrap();

    // 親HTMLでng-includeを定義
    let parent_html = r#"
<div ng-controller="ParentController">
    <div ng-controller="ChildController">
        <div ng-include="'child.html'"></div>
    </div>
</div>
"#;
    analyzer.analyze_document(&parent_uri, parent_html);

    // 子HTMLを解析
    let child_uri = Url::parse("file:///child.html").unwrap();
    let child_html = r#"
<div>
    <span>{{message}}</span>
</div>
"#;
    analyzer.analyze_document(&child_uri, child_html);

    // 子HTMLの参照が両方のコントローラーに対してget_all_referencesで見つかるか
    let parent_refs = index.get_all_references("ParentController.$scope.message");
    let child_refs = index.get_all_references("ChildController.$scope.message");

    assert!(!parent_refs.is_empty(), "message should be found for ParentController");
    assert!(!child_refs.is_empty(), "message should be found for ChildController");
}

#[rstest]
fn test_resolve_controllers_for_html_with_inheritance(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let parent_uri = Url::parse("file:///parent.html").unwrap();

    // 親HTMLでng-includeを定義
    let parent_html = r#"
<div ng-controller="OuterController">
    <div ng-controller="InnerController">
        <div ng-include="'included.html'"></div>
    </div>
</div>
"#;
    analyzer.analyze_document(&parent_uri, parent_html);

    // 子HTMLで追加のng-controllerがある場合
    let child_uri = Url::parse("file:///included.html").unwrap();
    let child_html = r#"
<div ng-controller="LocalController">
    <span>{{value}}</span>
</div>
"#;
    analyzer.analyze_document(&child_uri, child_html);

    // resolve_controllers_for_htmlが全てのコントローラーを返すか
    let controllers = index.resolve_controllers_for_html(&child_uri, 2);
    assert!(controllers.contains(&"OuterController".to_string()), "Should contain OuterController");
    assert!(controllers.contains(&"InnerController".to_string()), "Should contain InnerController");
    assert!(controllers.contains(&"LocalController".to_string()), "Should contain LocalController");
}

#[rstest]
fn test_data_ng_include_attribute(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///parent.html").unwrap();

    let html = r#"
<div ng-controller="TestController">
    <div data-ng-include="'template.html'"></div>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // data-ng-includeも検出されるか
    let child_uri = Url::parse("file:///template.html").unwrap();
    let inherited = index.get_inherited_controllers_for_template(&child_uri);
    assert_eq!(inherited.len(), 1, "Should inherit 1 controller");
    assert_eq!(inherited[0], "TestController");
}

#[rstest]
fn test_get_html_controllers_at_order(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="FirstController">
    <div ng-controller="SecondController">
        <div ng-controller="ThirdController">
            <span>Content</span>
        </div>
    </div>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // 全てのコントローラーが外側から内側の順で取得されるか
    let controllers = index.get_html_controllers_at(&uri, 4);
    assert_eq!(controllers.len(), 3);
    assert_eq!(controllers[0], "FirstController");
    assert_eq!(controllers[1], "SecondController");
    assert_eq!(controllers[2], "ThirdController");
}

#[rstest]
fn test_ng_if_outside_controller_scope(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    // ng-ifがaControllerの外側にある場合
    let html = r#"<div ng-if="status">
    <div ng-controller="aController">
        <span>{{innerValue}}</span>
    </div>
</div>"#;
    analyzer.analyze_document(&uri, html);

    // statusはaControllerの外側（行0）にある
    // aControllerは行1から始まる
    // statusはaControllerのスコープに含まれてはいけない
    let status_refs = index.get_all_references("aController.$scope.status");
    assert!(status_refs.is_empty(), "status should NOT be found for aController (it's outside the controller scope)");

    // innerValueはaControllerの内側（行2）にある
    let inner_refs = index.get_all_references("aController.$scope.innerValue");
    assert!(!inner_refs.is_empty(), "innerValue should be found for aController");
}

#[rstest]
fn test_ng_include_with_dynamic_path(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///parent.html").unwrap();

    // 動的なパス（文字列連結）を含むng-include
    let html = r#"
<div ng-controller="MainController">
    <div ng-include="'../static/wf/views/request_expense/request_expense_view.html?' + app_version"></div>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // 文字列リテラル部分が抽出されているか
    let child_uri = Url::parse("file:///request_expense_view.html").unwrap();
    let inherited = index.get_inherited_controllers_for_template(&child_uri);
    assert_eq!(inherited.len(), 1, "Should inherit 1 controller even with dynamic path");
    assert_eq!(inherited[0], "MainController");
}

#[rstest]
fn test_ng_include_with_query_param_and_version(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///parent.html").unwrap();

    // クエリパラメータ付きのパス
    let html = r#"
<div ng-controller="TestController">
    <div ng-include="'views/modal.html?v=' + version"></div>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // ファイル名部分でマッチするか（クエリパラメータは除去される）
    let child_uri = Url::parse("file:///modal.html").unwrap();
    let inherited = index.get_inherited_controllers_for_template(&child_uri);
    assert_eq!(inherited.len(), 1, "Should inherit 1 controller");
    assert_eq!(inherited[0], "TestController");
}

#[rstest]
fn test_ng_include_with_relative_path(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    // 親ファイルが /app/views/main.html にある場合
    let uri = Url::parse("file:///app/views/main.html").unwrap();

    // 相対パス ../static/wf/views/request/request_details.html
    let html = r#"
<div ng-controller="MainController">
    <div ng-include="'../static/wf/views/request/request_details.html?' + app_version"></div>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // 親ファイル /app/views/main.html を基準に解決すると
    // /app/static/wf/views/request/request_details.html になる
    // ファイル名は request_details.html
    let child_uri = Url::parse("file:///app/static/wf/views/request/request_details.html").unwrap();
    let inherited = index.get_inherited_controllers_for_template(&child_uri);
    assert_eq!(inherited.len(), 1, "Should inherit 1 controller via relative path resolution");
    assert_eq!(inherited[0], "MainController");
}

#[rstest]
fn test_ng_include_with_absolute_path(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///app/views/main.html").unwrap();

    // 絶対パス /static/templates/header.html
    let html = r#"
<div ng-controller="HeaderController">
    <div ng-include="'/static/templates/header.html'"></div>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // 絶対パスの場合はそのまま解決
    let child_uri = Url::parse("file:///static/templates/header.html").unwrap();
    let inherited = index.get_inherited_controllers_for_template(&child_uri);
    assert_eq!(inherited.len(), 1, "Should inherit 1 controller via absolute path");
    assert_eq!(inherited[0], "HeaderController");
}

#[rstest]
fn test_resolve_relative_path_function(ctx: TestContext) {
    use crate::index::SymbolIndex;

    // 基本的な相対パス解決
    let parent_uri = Url::parse("file:///app/views/main.html").unwrap();

    // ../を含むパス
    let result = SymbolIndex::resolve_relative_path(&parent_uri, "../static/test.html");
    assert_eq!(result, "test.html");

    // 複数の../を含むパス
    let result = SymbolIndex::resolve_relative_path(&parent_uri, "../../templates/modal.html");
    assert_eq!(result, "modal.html");

    // 単純な相対パス
    let result = SymbolIndex::resolve_relative_path(&parent_uri, "partials/header.html");
    assert_eq!(result, "header.html");

    // 絶対パス
    let result = SymbolIndex::resolve_relative_path(&parent_uri, "/static/footer.html");
    assert_eq!(result, "footer.html");

    // クエリパラメータ付き
    let result = SymbolIndex::resolve_relative_path(&parent_uri, "../views/detail.html?v=123");
    assert_eq!(result, "detail.html");
}

// ============================================================================
// wf_patterns: jbc-wf-container のパターンに基づくテスト
// ============================================================================

#[rstest]
fn test_wf_custom_bracket_interpolation(ctx: TestContext) {
    // jbc-wf-container では [[ ]] をinterpolation記号として使用
    let TestContext { analyzer, index, .. } = ctx;
    analyzer.set_interpolate_config(crate::config::InterpolateConfig {
        start_symbol: "[[".to_string(),
        end_symbol: "]]".to_string(),
    });

    let uri = Url::parse("file:///test.html").unwrap();
    let html = r#"
<div ng-controller="TestController">
    <span>[[ userName ]]</span>
    <span>[[ 'テキスト' | translate ]]</span>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // userNameへの参照が抽出されているか
    let refs = index.get_all_references("TestController.$scope.userName");
    assert!(!refs.is_empty(), "[[ userName ]] should be detected as scope reference");
}

#[rstest]
fn test_wf_bracket_interpolation_with_translate_filter(ctx: TestContext) {
    // [[ 'テキスト' | translate ]] パターン - 文字列リテラルのみの場合
    let TestContext { analyzer, index, .. } = ctx;
    analyzer.set_interpolate_config(crate::config::InterpolateConfig {
        start_symbol: "[[".to_string(),
        end_symbol: "]]".to_string(),
    });

    let uri = Url::parse("file:///test.html").unwrap();
    let html = r#"
<div ng-controller="TestController">
    <span>[[ 'クラウドサイン書類の編集' | translate ]]</span>
    <span>[[ row.amount | number ]] [['円' | translate]]</span>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // row.amountへの参照が抽出されているか
    let refs = index.get_all_references("TestController.$scope.row");
    assert!(!refs.is_empty(), "[[ row.amount | number ]] should detect 'row' as scope reference");
}

#[rstest]
fn test_wf_ng_repeat_tuple_unpacking(ctx: TestContext) {
    // ng-repeat="(i, item) in collection" パターン
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="CloudsignController">
    <ul>
        <li ng-repeat="(i, cloudsign_file) in doc.files">
            <a ng-click="deleteCloudsignPdf(i)">Delete</a>
            <span>{{ cloudsign_file.name }}</span>
        </li>
    </ul>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // doc.filesへの参照が抽出されているか
    let refs = index.get_all_references("CloudsignController.$scope.doc");
    assert!(!refs.is_empty(), "ng-repeat with tuple unpacking should detect 'doc' as scope reference");

    // deleteCloudsignPdfへの参照が抽出されているか
    let method_refs = index.get_all_references("CloudsignController.$scope.deleteCloudsignPdf");
    assert!(!method_refs.is_empty(), "ng-click should detect 'deleteCloudsignPdf' as scope reference");

    // iとcloudsign_fileはローカル変数なのでスコープ参照として登録されないべき
    let i_refs = index.get_all_references("CloudsignController.$scope.i");
    assert!(i_refs.is_empty(), "'i' is a local variable from ng-repeat, should not be a scope reference");
}

#[rstest]
fn test_wf_multiline_ng_if_condition(ctx: TestContext) {
    // 複数行にまたがるng-if条件
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="RequestController">
    <div ng-if="req.cloudsign_document.files !== undefined &&
                req.cloudsign_document.files !== null &&
                req.cloudsign_document.files.length !== 0">
        <span>Files exist</span>
    </div>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // reqへの参照が抽出されているか
    let refs = index.get_all_references("RequestController.$scope.req");
    assert!(!refs.is_empty(), "multiline ng-if should detect 'req' as scope reference");
}

#[rstest]
fn test_wf_dynamic_ng_include_path(ctx: TestContext) {
    // 動的なパスを含むng-include
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///app/views/main.html").unwrap();

    let html = r#"
<div ng-controller="MainController">
    <ng-include ng-if="showParticipants"
                src="'../static/wf/app/cloudsign/views/participants.html?' + app_version">
    </ng-include>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // showParticipantsへの参照が抽出されているか
    let refs = index.get_all_references("MainController.$scope.showParticipants");
    assert!(!refs.is_empty(), "ng-if on ng-include should detect 'showParticipants' as scope reference");

    // ng-includeのパスが解決されているか（ファイル名でマッチ）
    let child_uri = Url::parse("file:///participants.html").unwrap();
    let inherited = index.get_inherited_controllers_for_template(&child_uri);
    assert_eq!(inherited.len(), 1, "ng-include with dynamic path should inherit controller");
}

#[rstest]
fn test_wf_ngf_directive_scope_references(ctx: TestContext) {
    // ngf-drop, ngf-select ディレクティブ（angular-file-upload）
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="UploadController">
    <div ngf-drop="registerPdf($files, $invalidFiles)"
         ngf-select="registerPdf($files, $invalidFiles)"
         ngf-pattern="'.pdf'"
         ngf-max-size="10MB"
         multiple="multiple">
        Drop files here
    </div>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // registerPdfへの参照が抽出されているか
    let refs = index.get_all_references("UploadController.$scope.registerPdf");
    assert!(!refs.is_empty(), "ngf-drop/ngf-select should detect 'registerPdf' as scope reference");

    // 複数のngf-*ディレクティブから同じ関数が参照されている
    // ngf-dropとngf-selectの両方からregisterPdfが検出される
    assert!(refs.len() >= 2, "registerPdf should be detected from both ngf-drop and ngf-select");
}

#[rstest]
fn test_wf_uib_tooltip_with_translate(ctx: TestContext) {
    // uib-tooltip内でのtranslateフィルター使用
    let TestContext { analyzer, index, .. } = ctx;
    analyzer.set_interpolate_config(crate::config::InterpolateConfig {
        start_symbol: "[[".to_string(),
        end_symbol: "]]".to_string(),
    });

    let uri = Url::parse("file:///test.html").unwrap();
    let html = r#"
<div ng-controller="JournalController">
    <a uib-tooltip="[['ダウンロード' | translate]]" ng-click="download()">
        <i class="fa fa-download"></i>
    </a>
    <input uib-tooltip="[['クリア'|translate]]" ng-model="searchText">
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // download()への参照が抽出されているか
    let refs = index.get_all_references("JournalController.$scope.download");
    assert!(!refs.is_empty(), "ng-click should detect 'download' as scope reference");

    // searchTextへの参照が抽出されているか
    let model_refs = index.get_all_references("JournalController.$scope.searchText");
    assert!(!model_refs.is_empty(), "ng-model should detect 'searchText' as scope reference");
}

#[rstest]
fn test_wf_ng_messages_form_validation(ctx: TestContext) {
    // ng-messages フォームバリデーション
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="DialogController">
    <form name="dialog_form">
        <input name="allowance_code" ng-model="allowanceCode" required>
        <div ng-messages="dialog_form.allowance_code.$error"
             ng-message-multiple
             ng-show="dialog_form.allowance_code.$touched">
            <div ng-message="required">必須項目です</div>
        </div>
    </form>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // allowanceCodeへの参照が抽出されているか
    let refs = index.get_all_references("DialogController.$scope.allowanceCode");
    assert!(!refs.is_empty(), "ng-model should detect 'allowanceCode' as scope reference");

    // フォームバインディングの確認
    let form_binding = index.find_form_binding_definition(&uri, "dialog_form", 4);
    assert!(form_binding.is_some(), "dialog_form should be found as form binding");

    // ng-messages内のdialog_form参照
    // dialog_form.allowance_code の形式でHtmlScopeReferenceとして登録される
    // （$errorはAngularの内部プロパティなのでスコープ参照としては登録されない）
    let all_refs = index.html_scope_references_for_test(&uri).unwrap_or_default();
    let form_ref = all_refs.iter().find(|r| r.property_path == "dialog_form.allowance_code");
    assert!(form_ref.is_some(), "ng-messages should detect 'dialog_form.allowance_code' as scope reference");

    // dialog_form 単体の参照も検出される（ベース名として）
    let base_ref = all_refs.iter().find(|r| r.property_path == "dialog_form");
    assert!(base_ref.is_some(), "dialog_form should be detected as base name reference");
}

#[rstest]
fn test_wf_nested_ng_repeat_with_index(ctx: TestContext) {
    // ネストされたng-repeatとインデックス変数
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="TableController">
    <table>
        <tr ng-repeat="row in rows track by $index">
            <td ng-repeat="cell in row.cells track by $index">
                {{ cell.value }}
            </td>
            <td ng-click="deleteRow($index)">Delete</td>
        </tr>
    </table>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // rowsへの参照が抽出されているか
    let refs = index.get_all_references("TableController.$scope.rows");
    assert!(!refs.is_empty(), "ng-repeat should detect 'rows' as scope reference");

    // deleteRowへの参照が抽出されているか
    let method_refs = index.get_all_references("TableController.$scope.deleteRow");
    assert!(!method_refs.is_empty(), "ng-click with $index should detect 'deleteRow' as scope reference");

    // $indexはAngularの特殊変数なのでスコープ参照として登録されないべき
    let index_refs = index.get_all_references("TableController.$scope.$index");
    assert!(index_refs.is_empty(), "'$index' is a special ng-repeat variable, should not be a scope reference");
}

#[rstest]
fn test_wf_complex_ng_class_expression(ctx: TestContext) {
    // ng-classの複雑な式
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="StyleController">
    <tr ng-class="{'active': isActive, 'selected': row.selected, 'disabled': !canEdit}">
        <td>Content</td>
    </tr>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // isActiveへの参照が抽出されているか
    let refs = index.get_all_references("StyleController.$scope.isActive");
    assert!(!refs.is_empty(), "ng-class should detect 'isActive' as scope reference");

    // rowへの参照が抽出されているか
    let row_refs = index.get_all_references("StyleController.$scope.row");
    assert!(!row_refs.is_empty(), "ng-class should detect 'row' as scope reference");

    // canEditへの参照が抽出されているか
    let can_edit_refs = index.get_all_references("StyleController.$scope.canEdit");
    assert!(!can_edit_refs.is_empty(), "ng-class with negation should detect 'canEdit' as scope reference");
}

#[rstest]
fn test_wf_ng_options_complex_expression(ctx: TestContext) {
    // ng-optionsの複雑な式
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="SelectController">
    <select ng-model="selectedItem"
            ng-options="item.id as item.name for item in items track by item.id">
    </select>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // selectedItemへの参照が抽出されているか
    let refs = index.get_all_references("SelectController.$scope.selectedItem");
    assert!(!refs.is_empty(), "ng-model should detect 'selectedItem' as scope reference");

    // itemsへの参照が抽出されているか
    let items_refs = index.get_all_references("SelectController.$scope.items");
    assert!(!items_refs.is_empty(), "ng-options should detect 'items' as scope reference");
}

#[rstest]
fn test_wf_mouse_event_handlers(ctx: TestContext) {
    // マウスイベントハンドラー
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="TableController">
    <tr ng-repeat="billing in form_data"
        ng-click="linkBillingAddressDetail(billing.parent_billing_address)"
        ng-mouseenter="is_item_hovering = true"
        ng-mouseleave="is_item_hovering = false">
        <td>{{ billing.name }}</td>
    </tr>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // form_dataへの参照が抽出されているか
    let refs = index.get_all_references("TableController.$scope.form_data");
    assert!(!refs.is_empty(), "ng-repeat should detect 'form_data' as scope reference");

    // linkBillingAddressDetailへの参照が抽出されているか
    let method_refs = index.get_all_references("TableController.$scope.linkBillingAddressDetail");
    assert!(!method_refs.is_empty(), "ng-click should detect 'linkBillingAddressDetail' as scope reference");

    // is_item_hoveringへの参照が抽出されているか
    let hover_refs = index.get_all_references("TableController.$scope.is_item_hovering");
    assert!(!hover_refs.is_empty(), "ng-mouseenter/ng-mouseleave should detect 'is_item_hovering' as scope reference");
}

#[rstest]
fn test_ng_include_inheritance_chain_propagation(ctx: TestContext) {
    // 継承チェーンの伝播テスト
    // 子ファイルが先に解析されても、親が後で解析されたら継承情報が伝播される
    let TestContext { analyzer, index, .. } = ctx;

    // 1. 孫ファイル（grandchild.html）を先に解析
    let grandchild_uri = Url::parse("file:///static/wf/views/grandchild.html").unwrap();
    let grandchild_html = r#"<span>{{message}}</span>"#;
    analyzer.analyze_document(&grandchild_uri, grandchild_html);

    // 2. 子ファイル（child.html）を解析（grandchild.htmlをng-include）
    let child_uri = Url::parse("file:///static/wf/views/child.html").unwrap();
    let child_html = r#"
<div>
    <ng-include src="'../static/wf/views/grandchild.html'"></ng-include>
</div>
"#;
    analyzer.analyze_document(&child_uri, child_html);

    // この時点ではgrandchild.htmlへの継承は空
    let inherited = index.get_inherited_controllers_for_template(&grandchild_uri);
    assert!(inherited.is_empty(), "Grandchild should have no inheritance yet");

    // 3. 親ファイル（parent.html）を解析（child.htmlをng-include、ng-controllerあり）
    let parent_uri = Url::parse("file:///static/wf/views/parent.html").unwrap();
    let parent_html = r#"
<div ng-controller="ParentController">
    <ng-include src="'../static/wf/views/child.html'"></ng-include>
</div>
"#;
    analyzer.analyze_document(&parent_uri, parent_html);

    // 親が解析された後、子への継承が設定される
    let child_inherited = index.get_inherited_controllers_for_template(&child_uri);
    assert_eq!(child_inherited.len(), 1, "Child should inherit ParentController");
    assert_eq!(child_inherited[0], "ParentController");

    // 孫への継承も伝播される
    let grandchild_inherited = index.get_inherited_controllers_for_template(&grandchild_uri);
    assert_eq!(grandchild_inherited.len(), 1, "Grandchild should inherit ParentController through propagation");
    assert_eq!(grandchild_inherited[0], "ParentController");
}

#[rstest]
fn test_uibmodal_inheritance_propagation(ctx: TestContext) {
    // $uibModalでバインドされたコントローラーがng-includeの子に伝播されるテスト
    use crate::index::{BindingSource, TemplateBinding};

    let TestContext { analyzer, index, .. } = ctx;

    // 1. 子ファイル（custom_text.html）を先に解析（ng-includeあり）
    let child_uri = Url::parse("file:///static/wf/views/form/custom_item/custom_text.html").unwrap();
    let child_html = r#"
<div>
    <ng-include src="'../static/wf/views/form/custom_item/parts/item_name.html'"></ng-include>
</div>
"#;
    analyzer.analyze_document(&child_uri, child_html);

    // 2. 孫ファイル（item_name.html）を解析
    let grandchild_uri = Url::parse("file:///static/wf/views/form/custom_item/parts/item_name.html").unwrap();
    let grandchild_html = r#"<span>{{item_name}}</span>"#;
    analyzer.analyze_document(&grandchild_uri, grandchild_html);

    // この時点ではitem_name.htmlへの継承は空
    let inherited = index.get_inherited_controllers_for_template(&grandchild_uri);
    assert!(inherited.is_empty(), "Item_name should have no inheritance yet");

    // 3. $uibModal.open()でテンプレートバインディングを追加
    // （これはJSファイル解析時に呼ばれる）
    let binding = TemplateBinding {
        template_path: "../static/wf/views/form/custom_item/custom_text.html".to_string(),
        controller_name: "FormCustomItemDialogController".to_string(),
        source: BindingSource::UibModal,
    };
    index.add_template_binding(binding);

    // $uibModalバインディング追加後、孫への継承も伝播される
    let grandchild_inherited = index.get_inherited_controllers_for_template(&grandchild_uri);
    assert_eq!(grandchild_inherited.len(), 1, "Item_name should inherit FormCustomItemDialogController through propagation");
    assert_eq!(grandchild_inherited[0], "FormCustomItemDialogController");
}

#[rstest]
fn test_form_binding_inheritance_via_ng_include(ctx: TestContext) {
    // ng-include経由でフォームバインディングが継承されることをテスト
    let TestContext { analyzer, index, .. } = ctx;

    // 1. 親HTML（フォームを含み、子テンプレートをng-include）
    let parent_uri = Url::parse("file:///parent.html").unwrap();
    let parent_html = r#"
<div ng-controller="ParentController">
    <form name="myForm">
        <div ng-include="'child.html'"></div>
    </form>
</div>
"#;
    analyzer.analyze_document(&parent_uri, parent_html);

    // 2. 子HTML（親のフォームを参照）
    let child_uri = Url::parse("file:///child.html").unwrap();
    let child_html = r#"
<div>
    <input ng-model="inputValue">
    <div ng-show="myForm.inputField.$invalid">Error</div>
</div>
"#;
    analyzer.analyze_document(&child_uri, child_html);

    // 子テンプレートで継承されたフォームバインディングを確認
    let inherited_forms = index.get_inherited_form_bindings_for_template(&child_uri);
    assert_eq!(inherited_forms.len(), 1, "Child should inherit myForm from parent");
    assert_eq!(inherited_forms[0].name, "myForm");

    // find_form_binding_definitionでも継承されたフォームが見つかる
    let form_binding = index.find_form_binding_definition(&child_uri, "myForm", 3);
    assert!(form_binding.is_some(), "myForm should be found in child template via inheritance");
    assert_eq!(form_binding.unwrap().name, "myForm");

    // 子テンプレート内のフォーム参照がHtmlScopeReferenceとして登録されているか確認
    let child_refs = index.html_scope_references_for_test(&child_uri).unwrap_or_default();
    let form_ref = child_refs.iter().find(|r| r.property_path == "myForm.inputField");
    assert!(form_ref.is_some(), "myForm.inputField should be detected as scope reference in child template");
}

/// 子HTML→親HTMLの順で解析した場合のフォームバインディング継承テスト
#[rstest]
fn test_form_binding_inheritance_child_first(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;

    // 1. 子HTML（親のフォームを参照）を先に解析
    let child_uri = Url::parse("file:///child.html").unwrap();
    let child_html = r#"
<div>
    <input ng-model="inputValue">
    <div ng-show="myForm.inputField.$invalid">Error</div>
</div>
"#;
    analyzer.analyze_document(&child_uri, child_html);

    // この時点ではフォームバインディングは見つからない
    let form_before = index.find_form_binding_definition(&child_uri, "myForm", 3);
    eprintln!("form_before: {:?}", form_before);
    assert!(form_before.is_none(), "myForm should NOT be found before parent is analyzed");

    // 子テンプレート内のフォーム参照は登録されていない
    let child_refs_before = index.html_scope_references_for_test(&child_uri).unwrap_or_default();
    let form_ref_before = child_refs_before.iter().find(|r| r.property_path == "myForm.inputField");
    eprintln!("form_ref_before: {:?}", form_ref_before);
    assert!(form_ref_before.is_none(), "myForm.inputField should NOT be registered before parent is analyzed");

    // 2. 親HTML（フォームを含み、子テンプレートをng-include）を解析
    let parent_uri = Url::parse("file:///parent.html").unwrap();
    let parent_html = r#"
<div ng-controller="ParentController">
    <form name="myForm">
        <div ng-include="'child.html'"></div>
    </form>
</div>
"#;
    analyzer.analyze_document(&parent_uri, parent_html);

    // 親解析後、継承されたフォームバインディングが見つかる
    let inherited_forms = index.get_inherited_form_bindings_for_template(&child_uri);
    eprintln!("inherited_forms after parent: {:?}", inherited_forms);
    assert_eq!(inherited_forms.len(), 1, "Child should inherit myForm from parent");

    // 3. 子HTMLを再解析
    analyzer.analyze_document(&child_uri, child_html);

    // 再解析後、フォーム参照が登録される
    let child_refs_after = index.html_scope_references_for_test(&child_uri).unwrap_or_default();
    let form_ref_after = child_refs_after.iter().find(|r| r.property_path == "myForm.inputField");
    eprintln!("form_ref_after: {:?}", form_ref_after);
    assert!(form_ref_after.is_some(), "myForm.inputField should be registered after reanalysis");
}

#[rstest]
fn test_controller_as_alias_syntax(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    // "controller as alias"構文を使用
    let html = r#"
<div ng-controller="FormCustomItemController as formCustomItem">
    <input ng-model="formCustomItem.itemName">
    <span>{{formCustomItem.description}}</span>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // aliasが正しく検出されているか
    let controller = index.get_html_controller_at(&uri, 1);
    assert_eq!(controller, Some("FormCustomItemController".to_string()));

    // ng-modelからalias.property形式の参照が抽出されているか
    // formCustomItem.itemName は位置21から始まる
    let ref_opt = index.find_html_scope_reference_at(&uri, 2, 21);
    assert!(ref_opt.is_some(), "ng-model reference should be found");
    let ref_found = ref_opt.unwrap();
    assert_eq!(ref_found.property_path, "formCustomItem.itemName");

    // interpolationからもalias.property形式の参照が抽出されているか
    let ref_opt2 = index.find_html_scope_reference_at(&uri, 3, 13);
    assert!(ref_opt2.is_some(), "interpolation reference should be found");
    let ref_found2 = ref_opt2.unwrap();
    assert_eq!(ref_found2.property_path, "formCustomItem.description");
}

#[rstest]
fn test_controller_as_alias_without_alias(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    // aliasなしの通常のコントローラー
    let html = r#"
<div ng-controller="UserController">
    <input ng-model="user.name">
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // "user.name"の"user"だけが抽出される（aliasではないため）
    let ref_opt = index.find_html_scope_reference_at(&uri, 2, 21);
    assert!(ref_opt.is_some(), "ng-model reference should be found");
    let ref_found = ref_opt.unwrap();
    assert_eq!(ref_found.property_path, "user");
}

#[rstest]
fn test_resolve_controller_by_alias(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="FormCustomItemController as formCustomItem">
    <span>Content</span>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // aliasからコントローラーを解決
    let controller = index.resolve_controller_by_alias(&uri, 2, "formCustomItem");
    assert_eq!(controller, Some("FormCustomItemController".to_string()));

    // 存在しないaliasは解決できない
    let not_found = index.resolve_controller_by_alias(&uri, 2, "notAnAlias");
    assert!(not_found.is_none());
}

#[rstest]
fn test_nested_controller_as_alias(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    // ネストされたcontroller as alias構文
    let html = r#"
<div ng-controller="OuterController as outer">
    <div ng-controller="InnerController as inner">
        <span>{{inner.value}}</span>
        <span>{{outer.data}}</span>
    </div>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // 内側のコントローラーのalias
    let inner_controller = index.resolve_controller_by_alias(&uri, 3, "inner");
    assert_eq!(inner_controller, Some("InnerController".to_string()));

    // 外側のコントローラーのalias
    let outer_controller = index.resolve_controller_by_alias(&uri, 3, "outer");
    assert_eq!(outer_controller, Some("OuterController".to_string()));
}

#[rstest]
fn test_data_ng_controller_as_alias(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    // data-ng-controller形式でのcontroller as alias構文
    let html = r#"
<div data-ng-controller="TestController as vm">
    <input ng-model="vm.username">
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // aliasが正しく検出されているか
    let controller = index.get_html_controller_at(&uri, 1);
    assert_eq!(controller, Some("TestController".to_string()));

    // aliasからコントローラーを解決
    let resolved = index.resolve_controller_by_alias(&uri, 2, "vm");
    assert_eq!(resolved, Some("TestController".to_string()));

    // alias.property形式の参照が抽出されているか
    let ref_opt = index.find_html_scope_reference_at(&uri, 2, 21);
    assert!(ref_opt.is_some());
    assert_eq!(ref_opt.unwrap().property_path, "vm.username");
}

/// controller as 構文で関数呼び出し形式（alias.method(args)）の参照が抽出されることを確認
#[rstest]
fn test_controller_as_alias_with_function_call(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="FormCustomItemController as formCustomItem">
    <input ng-pattern="formCustomItem.getInputPattern(form_custom_item)">
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // aliasが正しく検出されているか
    let controller = index.get_html_controller_at(&uri, 1);
    assert_eq!(controller, Some("FormCustomItemController".to_string()));

    // aliasからコントローラーを解決
    let resolved = index.resolve_controller_by_alias(&uri, 2, "formCustomItem");
    assert_eq!(resolved, Some("FormCustomItemController".to_string()));

    // 関数呼び出し形式でもalias.method形式の参照が抽出されているか
    // ng-pattern="formCustomItem.getInputPattern(form_custom_item)"
    // formCustomItem は位置 23 から始まる
    let ref_opt = index.find_html_scope_reference_at(&uri, 2, 23);
    assert!(ref_opt.is_some(), "formCustomItem.getInputPattern の参照が見つかるべき（関数呼び出し形式）");
    let ref_found = ref_opt.unwrap();
    assert_eq!(ref_found.property_path, "formCustomItem.getInputPattern");
}

/// controller as 構文で $scope.xxx パターンを使用したコントローラーに対する
/// HTMLからの定義ジャンプが動作することを確認
#[rstest]
fn test_controller_as_with_scope_pattern_definition_lookup(ctx: TestContext) {
    let TestContext { analyzer: html_analyzer, index, .. } = ctx;

    // JSファイルでコントローラーを定義（$scope.xxx パターン）
    let js_source = r#"
angular.module('app')
.controller('FormCustomItemController', ['$scope', function($scope) {
    $scope.itemName = '';
    $scope.getInputPattern = function(item) {
        return /^[a-z]+$/;
    };
}]);
"#;
    let js_uri = Url::parse("file:///controller.js").unwrap();

    // JSアナライザーでコントローラーを解析
    let js_analyzer = AngularJsAnalyzer::new(index.clone());
    js_analyzer.analyze_document_with_options(&js_uri, js_source, true);

    // HTMLファイルでcontroller as構文を使用
    let html = r#"
<div ng-controller="FormCustomItemController as formCustomItem">
    <input ng-model="formCustomItem.itemName">
    <input ng-pattern="formCustomItem.getInputPattern(item)">
</div>
"#;
    let html_uri = Url::parse("file:///template.html").unwrap();
    html_analyzer.analyze_document(&html_uri, html);

    // formCustomItem.itemName の参照を確認（位置21）
    let item_name_ref = index.find_html_scope_reference_at(&html_uri, 2, 21);
    assert!(item_name_ref.is_some(), "formCustomItem.itemName の参照が見つかるべき");
    assert_eq!(item_name_ref.unwrap().property_path, "formCustomItem.itemName");

    // formCustomItem.getInputPattern の参照を確認（位置23）
    let method_ref = index.find_html_scope_reference_at(&html_uri, 3, 23);
    assert!(method_ref.is_some(), "formCustomItem.getInputPattern の参照が見つかるべき");
    assert_eq!(method_ref.unwrap().property_path, "formCustomItem.getInputPattern");

    // aliasからコントローラー名を解決
    let controller = index.resolve_controller_by_alias(&html_uri, 2, "formCustomItem");
    assert_eq!(controller, Some("FormCustomItemController".to_string()));

    // JSで定義された $scope.itemName が検索できる
    let item_defs = index.get_definitions("FormCustomItemController.$scope.itemName");
    assert!(!item_defs.is_empty(), "$scope.itemName の定義が FormCustomItemController.$scope.itemName として登録されるべき");

    // JSで定義された $scope.getInputPattern が検索できる
    let method_defs = index.get_definitions("FormCustomItemController.$scope.getInputPattern");
    assert!(!method_defs.is_empty(), "$scope.getInputPattern の定義が FormCustomItemController.$scope.getInputPattern として登録されるべき");
}


#[rstest]
fn test_ng_class_complex_expression_with_subscript(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    // 複雑なng-class式（subscript expressionを含む）
    let html = r#"
<div ng-controller="FormController">
<label ng-class="{'required-error': requestForm.input_name.$invalid && clicked_confirm,
  'modified': !canEditForm() && modifyRequestFormCustomItem.item_list[form_custom_item_idx].modified}">
</label>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // 抽出された参照を確認
    let refs = index.get_all_references("FormController.$scope.requestForm");
    assert!(!refs.is_empty(), "ng-class should detect 'requestForm' as scope reference");

    let refs2 = index.get_all_references("FormController.$scope.clicked_confirm");
    assert!(!refs2.is_empty(), "ng-class should detect 'clicked_confirm' as scope reference");

    let refs3 = index.get_all_references("FormController.$scope.canEditForm");
    assert!(!refs3.is_empty(), "ng-class should detect 'canEditForm' as scope reference");

    let refs4 = index.get_all_references("FormController.$scope.modifyRequestFormCustomItem");
    assert!(!refs4.is_empty(), "ng-class should detect 'modifyRequestFormCustomItem' as scope reference");

    let refs5 = index.get_all_references("FormController.$scope.form_custom_item_idx");
    assert!(!refs5.is_empty(), "ng-class should detect 'form_custom_item_idx' as scope reference");

    // 位置情報の確認 - clicked_confirmでGo to Definitionできるか
    // ng-class="{'required-error': requestForm.input_name.$invalid && clicked_confirm,
    // clicked_confirmは行2のcol 71から始まる
    let html_ref = index.find_html_scope_reference_at(&uri, 2, 71);
    assert!(html_ref.is_some(), "Should find reference at clicked_confirm position (col 71)");
    assert_eq!(html_ref.unwrap().property_path, "clicked_confirm");

    // requestFormも確認 (col 36)
    let html_ref2 = index.find_html_scope_reference_at(&uri, 2, 36);
    assert!(html_ref2.is_some(), "Should find reference at requestForm position");
    assert_eq!(html_ref2.unwrap().property_path, "requestForm");

    // modifyRequestFormCustomItemは3行目に（マルチライン属性値）
    //   'modified': !canEditForm() && modifyRequestFormCustomItem.item_list[...]
    //                                 ^-- col 32 から始まる
    let html_ref3 = index.find_html_scope_reference_at(&uri, 3, 32);
    assert!(html_ref3.is_some(), "Should find reference at modifyRequestFormCustomItem position (line 3, col 32)");
    assert_eq!(html_ref3.unwrap().property_path, "modifyRequestFormCustomItem");

    // form_custom_item_idxも3行目に
    //   modifyRequestFormCustomItem.item_list[form_custom_item_idx].modified
    //                                         ^-- col 70 から始まる
    let html_ref4 = index.find_html_scope_reference_at(&uri, 3, 70);
    assert!(html_ref4.is_some(), "Should find reference at form_custom_item_idx position (line 3, col 70)");
    assert_eq!(html_ref4.unwrap().property_path, "form_custom_item_idx");
}

#[rstest]
fn test_ng_class_with_interpolate_inside(ctx: TestContext) {
    // ng-class属性値内にカスタムinterpolate記号がある場合
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    // カスタムinterpolate記号を設定
    analyzer.set_interpolate_config(crate::config::InterpolateConfig {
        start_symbol: "[[".to_string(),
        end_symbol: "]]".to_string(),
    });

    // ユーザーの実際のケースに近いHTML
    let html = r#"
<div ng-controller="FormController">
<label ng-class="{'required-error': requestForm.[[form_custom_item.input_name]].$invalid===true && clicked_confirm,
  'modified': !canEditForm() && modifyRequestFormCustomItem.item_list[form_custom_item_idx].modified}">
</label>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // clicked_confirmへの参照が抽出されているか
    let refs = index.get_all_references("FormController.$scope.clicked_confirm");
    assert!(!refs.is_empty(), "ng-class should detect 'clicked_confirm' as scope reference");
}

#[rstest]
fn test_ng_class_vs_ng_if_position(ctx: TestContext) {
    // ng-classとng-ifで位置計算が正しいか比較
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="FormController">
    <span ng-if="modifyRequestFormCustomItem.value">A</span>
    <span ng-class="{'active': modifyRequestFormCustomItem.enabled}">B</span>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // 行内容を確認
    let lines: Vec<&str> = html.lines().collect();
    eprintln!("Line 2 (ng-if): '{}'", lines.get(2).unwrap_or(&""));
    eprintln!("Line 3 (ng-class): '{}'", lines.get(3).unwrap_or(&""));

    // HTML内の全ての参照を確認
    if let Some(all_refs) = index.html_scope_references_for_test(&uri) {
        eprintln!("All HTML scope references:");
        for r in &all_refs {
            eprintln!("  {} at line {}, col {}-{}", r.property_path, r.start_line, r.start_col, r.end_col);
        }
    }

    // ng-if内の参照を確認 - "modifyRequestFormCustomItem" は行2
    // ng-if="modifyRequestFormCustomItem.value"
    //        ^-- col 17 から始まるはず
    let ng_if_ref = index.find_html_scope_reference_at(&uri, 2, 17);
    eprintln!("ng-if ref at (2, 17): {:?}", ng_if_ref.as_ref().map(|r| &r.property_path));
    assert!(ng_if_ref.is_some(), "Should find reference in ng-if");
    assert_eq!(ng_if_ref.unwrap().property_path, "modifyRequestFormCustomItem");

    // ng-class内の参照を確認 - "modifyRequestFormCustomItem" は行3
    // ng-class="{'active': modifyRequestFormCustomItem.enabled}"
    //     <span ng-class="{'active': modifyRequestFormCustomItem...
    // 0         1         2         3
    // 0123456789012345678901234567890123456789
    //                               ^-- col 31 から始まる
    let ng_class_ref = index.find_html_scope_reference_at(&uri, 3, 31);
    eprintln!("ng-class ref at (3, 31): {:?}", ng_class_ref.as_ref().map(|r| &r.property_path));
    assert!(ng_class_ref.is_some(), "Should find reference in ng-class");
    assert_eq!(ng_class_ref.unwrap().property_path, "modifyRequestFormCustomItem");

    // 位置31でも見つかることを確認（範囲内）
    let ng_class_ref_mid = index.find_html_scope_reference_at(&uri, 3, 40);
    assert!(ng_class_ref_mid.is_some(), "Should find reference at middle of identifier");

    // 位置30（範囲外）では見つからないことを確認
    let ng_class_ref_before = index.find_html_scope_reference_at(&uri, 3, 30);
    assert!(ng_class_ref_before.is_none(), "Should NOT find reference before identifier");
}

#[rstest]
fn test_ng_class_multiline_position(ctx: TestContext) {
    // マルチラインng-classの位置計算
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    // ユーザーの実際のケースに近いマルチラインng-class
    let html = r#"
<div ng-controller="FormController">
<label ng-class="{'required-error': clicked_confirm,
  'modified': modifyRequestFormCustomItem.modified}">
</label>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // 行内容を確認
    let lines: Vec<&str> = html.lines().collect();
    eprintln!("Line 2: '{}'", lines.get(2).unwrap_or(&""));
    eprintln!("Line 3: '{}'", lines.get(3).unwrap_or(&""));

    // HTML内の全ての参照を確認
    if let Some(all_refs) = index.html_scope_references_for_test(&uri) {
        eprintln!("All HTML scope references:");
        for r in &all_refs {
            eprintln!("  {} at line {}, col {}-{}", r.property_path, r.start_line, r.start_col, r.end_col);
        }
    }

    // 1行目のclicked_confirmを確認
    // ng-class="{'required-error': clicked_confirm,
    // 0         1         2         3         4
    // 01234567890123456789012345678901234567890123456789
    //                                ^-- col 36 から始まる？
    let ref1 = index.find_html_scope_reference_at(&uri, 2, 36);
    eprintln!("ref at (2, 36): {:?}", ref1.as_ref().map(|r| &r.property_path));

    // 2行目のmodifyRequestFormCustomItemを確認
    //   'modified': modifyRequestFormCustomItem.modified}">
    // ^-- この行でcol 14から始まるはず
    let ref2 = index.find_html_scope_reference_at(&uri, 3, 14);
    eprintln!("ref at (3, 14): {:?}", ref2.as_ref().map(|r| &r.property_path));

    // 参照が2行目の正しい位置に登録されているか確認
    assert!(ref1.is_some() || ref2.is_some(), "At least one reference should be found");
}

/// controller as構文でthis.methodパターンを使用した場合の定義検索テスト
///
/// ```javascript
/// .controller('FormCustomItemController', function() {
///     this.onChangeText = function(item) { ... };
/// })
/// ```
/// と定義されている場合、HTMLの `formCustomItem.onChangeText` からジャンプできるべき
#[rstest]
fn test_controller_as_with_this_method_pattern(ctx: TestContext) {
    let TestContext { analyzer: html_analyzer, index, .. } = ctx;

    // JSファイルでコントローラーを定義（this.xxx パターン）
    let js_source = r#"
angular.module('app')
.controller('FormCustomItemController', function() {
    this.onChangeText = function(item) {
        console.log(item);
    };
    this.getInputPattern = function(item) {
        return /^[a-z]+$/;
    };
});
"#;
    let js_uri = Url::parse("file:///controller.js").unwrap();

    // JSアナライザーでコントローラーを解析
    let js_analyzer = AngularJsAnalyzer::new(index.clone());
    js_analyzer.analyze_document_with_options(&js_uri, js_source, true);

    // HTMLファイルでcontroller as構文を使用
    let html = r#"
<div ng-controller="FormCustomItemController as formCustomItem">
    <input ng-change="formCustomItem.onChangeText(form_custom_item)">
    <input ng-pattern="formCustomItem.getInputPattern(item)">
</div>
"#;
    let html_uri = Url::parse("file:///template.html").unwrap();
    html_analyzer.analyze_document(&html_uri, html);

    // formCustomItem.onChangeText の参照を確認
    let method_ref = index.find_html_scope_reference_at(&html_uri, 2, 23);
    assert!(method_ref.is_some(), "formCustomItem.onChangeText の参照が見つかるべき");
    assert_eq!(method_ref.unwrap().property_path, "formCustomItem.onChangeText");

    // aliasからコントローラー名を解決
    let controller = index.resolve_controller_by_alias(&html_uri, 2, "formCustomItem");
    assert_eq!(controller, Some("FormCustomItemController".to_string()));

    // JSで定義された this.onChangeText が FormCustomItemController.onChangeText として検索できる
    let method_defs = index.get_definitions("FormCustomItemController.onChangeText");
    assert!(!method_defs.is_empty(), "this.onChangeText の定義が FormCustomItemController.onChangeText として登録されるべき");

    // 定義の位置が正しいか確認（JSの3行目、this.onChangeText の部分）
    let def = &method_defs[0];
    assert_eq!(def.uri.path(), "/controller.js");
    // this.onChangeText は3行目（0-indexed で 3）
    assert_eq!(def.start_line, 3, "定義は3行目であるべき");

    // this.getInputPattern も同様に登録されているか確認
    let pattern_defs = index.get_definitions("FormCustomItemController.getInputPattern");
    assert!(!pattern_defs.is_empty(), "this.getInputPattern の定義が FormCustomItemController.getInputPattern として登録されるべき");
}

/// controller as構文で `const vm = this;` パターンを使用した場合の定義検索テスト
///
/// ```javascript
/// .controller('FormCustomItemController', function() {
///     const vm = this;
///     vm.onChangeText = onChangeText;
///     vm.getInputPattern = getInputPattern;
/// })
/// ```
/// と定義されている場合、HTMLの `formCustomItem.onChangeText` からジャンプできるべき
#[rstest]
fn test_controller_as_with_vm_alias_pattern(ctx: TestContext) {
    let TestContext { analyzer: html_analyzer, index, .. } = ctx;

    // JSファイルでコントローラーを定義（const vm = this; パターン）
    let js_source = r#"
angular.module('app')
.controller('FormCustomItemController', function() {
    const vm = this;

    vm.onChangeText = onChangeText;
    vm.getInputPattern = getInputPattern;

    function onChangeText(item) {
        console.log(item);
    }

    function getInputPattern(item) {
        return /^[a-z]+$/;
    }
});
"#;
    let js_uri = Url::parse("file:///controller.js").unwrap();

    // JSアナライザーでコントローラーを解析
    let js_analyzer = AngularJsAnalyzer::new(index.clone());
    js_analyzer.analyze_document_with_options(&js_uri, js_source, true);

    // HTMLファイルでcontroller as構文を使用
    let html = r#"
<div ng-controller="FormCustomItemController as formCustomItem">
    <input ng-change="formCustomItem.onChangeText(form_custom_item)">
    <input ng-pattern="formCustomItem.getInputPattern(item)">
</div>
"#;
    let html_uri = Url::parse("file:///template.html").unwrap();
    html_analyzer.analyze_document(&html_uri, html);

    // aliasからコントローラー名を解決
    let controller = index.resolve_controller_by_alias(&html_uri, 2, "formCustomItem");
    assert_eq!(controller, Some("FormCustomItemController".to_string()));

    // JSで定義された vm.onChangeText が FormCustomItemController.onChangeText として検索できる
    let method_defs = index.get_definitions("FormCustomItemController.onChangeText");
    assert!(!method_defs.is_empty(), "vm.onChangeText の定義が FormCustomItemController.onChangeText として登録されるべき");

    // 定義の位置が正しいか確認（JSの5行目、vm.onChangeText の部分）
    let def = &method_defs[0];
    assert_eq!(def.uri.path(), "/controller.js");
    // vm.onChangeText は5行目（0-indexed で 5）
    assert_eq!(def.start_line, 5, "定義は5行目であるべき");

    // vm.getInputPattern も同様に登録されているか確認
    let pattern_defs = index.get_definitions("FormCustomItemController.getInputPattern");
    assert!(!pattern_defs.is_empty(), "vm.getInputPattern の定義が FormCustomItemController.getInputPattern として登録されるべき");

    // 参照検索でHTML内の参照が見つかることを確認
    let all_refs = index.get_all_references("FormCustomItemController.onChangeText");
    assert!(!all_refs.is_empty(), "FormCustomItemController.onChangeText の参照がHTML内で見つかるべき");

    // 参照の位置がHTML内の正しい位置を指しているか確認
    let html_ref = all_refs.iter().find(|r| r.uri.path() == "/template.html");
    assert!(html_ref.is_some(), "HTML内の参照が見つかるべき");
}

/// ng-include経由で継承されたローカル変数への参照が正しく登録され、
/// 定義ジャンプができることを確認するテスト
#[rstest]
fn test_inherited_local_variable_reference(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;

    // 親テンプレート: ng-repeatで"sheet"を定義し、ng-includeで子テンプレートを読み込む
    let parent_html = r#"
<div ng-repeat="sheet in sheets">
  <ng-include src="'child.html'"></ng-include>
</div>
"#;
    let parent_uri = Url::parse("file:///parent.html").unwrap();
    analyzer.analyze_document(&parent_uri, parent_html);

    // 子テンプレート: 継承されたローカル変数"sheet"を参照
    let child_html = r#"
<div ng-repeat="row in sheet.rows">
  <span>[[row.name]]</span>
</div>
"#;
    let child_uri = Url::parse("file:///child.html").unwrap();
    analyzer.analyze_document(&child_uri, child_html);

    // 親テンプレートで"sheet"のローカル変数定義が登録されているか確認
    let parent_var = index.find_local_variable_definition(&parent_uri, "sheet", 1);
    assert!(parent_var.is_some(), "親テンプレートにsheetの定義があるべき");

    // 子テンプレートで継承されたローカル変数"sheet"が検索できるか確認
    let inherited_var = index.find_local_variable_definition(&child_uri, "sheet", 1);
    assert!(inherited_var.is_some(), "子テンプレートで継承されたsheetが見つかるべき");

    // 継承された変数の定義位置が親テンプレートを指しているか確認
    let var = inherited_var.unwrap();
    assert_eq!(var.uri.path(), "/parent.html", "継承された変数の定義は親テンプレートにあるべき");
    assert_eq!(var.name, "sheet");

    // 子テンプレートでローカル変数参照が登録されているか確認
    // "sheet.rows" の "sheet" の位置（ng-repeat="row in sheet.rows"）
    let local_var_ref = index.find_html_local_variable_at(&child_uri, 1, 24);
    assert!(local_var_ref.is_some(), "子テンプレートでsheetへの参照が登録されているべき");
    assert_eq!(local_var_ref.unwrap().variable_name, "sheet");
}

/// 実際のユースケース: 複雑なパスでのng-include継承
#[rstest]
fn test_inherited_local_variable_with_complex_path(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;

    // 親テンプレート: ng-repeatで"sheet"を定義し、ng-includeで子テンプレートを読み込む
    let parent_html = r#"
<div ng-repeat="sheet in req.request_expense.request_expense_specifics" ng-if="sheet.is_contained_attachment_of_receipt">
  <ng-include ng-if="sheet.specifics_type=='exchange_transport'" src="'../static/wf/views/request_expense/print/request_expense_exchange_transport_for_print_attachment_of_receipt.html?' + app_version"></ng-include>
</div>
"#;
    let parent_uri = Url::parse("file:///project/static/wf/views/request_expense/print/parent.html").unwrap();
    analyzer.analyze_document(&parent_uri, parent_html);

    // ng-includeバインディングが正しく登録されているか確認
    let inherited_vars = index.get_inherited_local_variables_for_template(
        &Url::parse("file:///project/static/wf/views/request_expense/print/request_expense_exchange_transport_for_print_attachment_of_receipt.html").unwrap()
    );
    eprintln!("inherited_vars for child: {:?}", inherited_vars);
    assert!(!inherited_vars.is_empty(), "子テンプレートに継承される変数があるべき");
    assert!(inherited_vars.iter().any(|v| v.name == "sheet"), "sheetが継承されるべき");

    // 子テンプレート
    let child_html = r#"
<div ng-repeat="row in sheet.request_expense_specifics_exchange_transport">
  <span>[[row.specifics_row_number]]</span>
</div>
"#;
    let child_uri = Url::parse("file:///project/static/wf/views/request_expense/print/request_expense_exchange_transport_for_print_attachment_of_receipt.html").unwrap();
    analyzer.analyze_document(&child_uri, child_html);

    // 子テンプレートでsheetへのローカル変数参照が登録されているか確認
    let local_var_ref = index.find_html_local_variable_at(&child_uri, 1, 24);
    eprintln!("local_var_ref at 1:24 = {:?}", local_var_ref);
    assert!(local_var_ref.is_some(), "子テンプレートでsheetへの参照が登録されているべき");

    // 定義ジャンプができるか確認
    let var_def = index.find_local_variable_definition(&child_uri, "sheet", 1);
    eprintln!("var_def = {:?}", var_def);
    assert!(var_def.is_some(), "sheetの定義が見つかるべき");
    let def = var_def.unwrap();
    assert_eq!(def.uri.path(), "/project/static/wf/views/request_expense/print/parent.html", "定義は親テンプレートにあるべき");
    assert_eq!(def.name, "sheet");
}

/// 実際のユースケース: request_expense_for_print_attachment_of_receipt.html
#[rstest]
fn test_inherited_local_variable_real_case(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;

    // 親テンプレート（実際のファイルパスを使用）
    let parent_html = r#"<div style="width: 100%;" ng-repeat="sheet in req.request_expense.request_expense_specifics" ng-if="sheet.is_contained_attachment_of_receipt">
  <ng-include ng-if="sheet.specifics_type=='normal'" src="'../static/wf/views/request_expense/print/request_expense_normal_for_print_attachment_of_receipt.html?' + app_version"></ng-include>
  <ng-include ng-if="sheet.specifics_type=='transport'" src="'../static/wf/views/request_expense/print/request_expense_transport_for_print_attachment_of_receipt.html?' + app_version"></ng-include>
  <ng-include ng-if="sheet.specifics_type=='entertainment'" src="'../static/wf/views/request_expense/print/request_expense_entertainment_for_print_attachment_of_receipt.html?' + app_version"></ng-include>
  <ng-include ng-if="sheet.specifics_type=='allowance'" src="'../static/wf/views/request_expense/print/request_expense_allowance_for_print_attachment_of_receipt.html?' + app_version"></ng-include>
  <ng-include ng-if="sheet.specifics_type=='exchange_normal'" src="'../static/wf/views/request_expense/print/request_expense_exchange_normal_for_print_attachment_of_receipt.html?' + app_version"></ng-include>
  <ng-include ng-if="sheet.specifics_type=='exchange_transport'" src="'../static/wf/views/request_expense/print/request_expense_exchange_transport_for_print_attachment_of_receipt.html?' + app_version"></ng-include>
  <ng-include ng-if="sheet.specifics_type=='custom_specifics'" src="'../static/wf/views/request_custom_specific/request_custom_specific_for_print_attachment_of_receipt.html?' + app_version"></ng-include>
</div>"#;
    let parent_uri = Url::parse("file:///home/mochi/workspace/jbc-wf-container/wfapp/src/static/wf/views/request_expense/print/request_expense_for_print_attachment_of_receipt.html").unwrap();
    analyzer.analyze_document(&parent_uri, parent_html);

    // 子テンプレート
    let child_html = r#"<div ng-repeat="row in sheet.request_expense_specifics_exchange_transport">
  <div class="receipt_table">
    <div class="attached">[[ '領収書添付欄' | translate]]</div>
  </div>
</div>"#;
    let child_uri = Url::parse("file:///home/mochi/workspace/jbc-wf-container/wfapp/src/static/wf/views/request_expense/print/request_expense_exchange_transport_for_print_attachment_of_receipt.html").unwrap();

    // 継承される変数を確認
    let inherited_vars = index.get_inherited_local_variables_for_template(&child_uri);
    eprintln!("inherited_vars for real case: {:?}", inherited_vars);
    assert!(!inherited_vars.is_empty(), "子テンプレートに継承される変数があるべき");

    analyzer.analyze_document(&child_uri, child_html);

    // sheetへのローカル変数参照が登録されているか確認
    // "row in sheet.request_expense..." の "sheet" の位置を確認
    // `<div ng-repeat="row in sheet.request_expense_specifics_exchange_transport">`
    // 0         1         2         3
    // 0123456789012345678901234567890123456789
    // <div ng-repeat="row in sheet
    // sheet は位置 23 から始まる
    let local_var_ref = index.find_html_local_variable_at(&child_uri, 0, 23);
    eprintln!("local_var_ref at 0:23 = {:?}", local_var_ref);

    // すべてのローカル変数参照をダンプ
    let all_refs = index.get_local_variable_references(&child_uri, "sheet", 0, 100);
    eprintln!("all sheet refs in child: {:?}", all_refs);

    assert!(local_var_ref.is_some(), "sheetへの参照が登録されているべき");

    // 定義ジャンプができるか確認
    let var_def = index.find_local_variable_definition(&child_uri, "sheet", 0);
    eprintln!("var_def = {:?}", var_def);
    assert!(var_def.is_some(), "sheetの定義が見つかるべき");
}

/// 子テンプレートが先に解析された場合のテスト
/// 親テンプレートが解析されるまでは継承情報がないため、子テンプレートを再解析する必要がある
#[rstest]
fn test_inherited_local_variable_child_first(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;

    // 子テンプレートを先に解析（この時点では継承情報がない）
    let child_html = r#"
<div ng-repeat="row in sheet.request_expense_specifics_exchange_transport">
  <span>[[row.specifics_row_number]]</span>
</div>
"#;
    let child_uri = Url::parse("file:///project/static/wf/views/request_expense/print/request_expense_exchange_transport_for_print_attachment_of_receipt.html").unwrap();
    analyzer.analyze_document(&child_uri, child_html);

    // この時点ではsheetへのローカル変数参照は登録されていない（継承情報がないため）
    let local_var_ref_before = index.find_html_local_variable_at(&child_uri, 1, 24);
    eprintln!("local_var_ref BEFORE parent: {:?}", local_var_ref_before);

    // スコープ参照として登録されているか確認
    let scope_ref_before = index.find_html_scope_reference_at(&child_uri, 1, 24);
    eprintln!("scope_ref BEFORE parent: {:?}", scope_ref_before);

    // 親テンプレートを解析
    let parent_html = r#"
<div ng-repeat="sheet in req.request_expense.request_expense_specifics" ng-if="sheet.is_contained_attachment_of_receipt">
  <ng-include ng-if="sheet.specifics_type=='exchange_transport'" src="'../static/wf/views/request_expense/print/request_expense_exchange_transport_for_print_attachment_of_receipt.html?' + app_version"></ng-include>
</div>
"#;
    let parent_uri = Url::parse("file:///project/static/wf/views/request_expense/print/parent.html").unwrap();
    analyzer.analyze_document(&parent_uri, parent_html);

    // 子テンプレートを再解析（継承情報が利用可能になった）
    analyzer.analyze_document(&child_uri, child_html);

    // 再解析後はsheetへのローカル変数参照が登録される
    let local_var_ref_after = index.find_html_local_variable_at(&child_uri, 1, 24);
    eprintln!("local_var_ref AFTER parent: {:?}", local_var_ref_after);
    assert!(local_var_ref_after.is_some(), "再解析後にsheetへの参照が登録されるべき");

    // 定義ジャンプができるか確認
    let var_def = index.find_local_variable_definition(&child_uri, "sheet", 1);
    assert!(var_def.is_some(), "sheetの定義が見つかるべき");
}

/// 親テンプレートの変数に対する参照検索で、子テンプレート内の参照も含まれることを確認
#[rstest]
fn test_inherited_local_variable_references_include_children(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;

    // 親テンプレート
    let parent_html = r#"
<div ng-repeat="sheet in sheets">
  <ng-include src="'child.html'"></ng-include>
</div>
"#;
    let parent_uri = Url::parse("file:///parent.html").unwrap();
    analyzer.analyze_document(&parent_uri, parent_html);

    // 子テンプレート
    let child_html = r#"
<div ng-repeat="row in sheet.rows">
  <span>[[sheet.name]]</span>
</div>
"#;
    let child_uri = Url::parse("file:///child.html").unwrap();
    analyzer.analyze_document(&child_uri, child_html);

    // 親テンプレートで定義されたsheetの参照を収集
    let parent_refs = index.get_local_variable_references(&parent_uri, "sheet", 1, 3);
    eprintln!("parent_refs: {:?}", parent_refs);

    // 継承先の参照も収集
    let inherited_refs = index.get_inherited_local_variable_references(&parent_uri, "sheet");
    eprintln!("inherited_refs: {:?}", inherited_refs);

    // 子テンプレート内のsheetへの参照が含まれているべき
    assert!(!inherited_refs.is_empty(), "子テンプレート内のsheetへの参照があるべき");
    assert!(inherited_refs.iter().all(|r| r.uri.path() == "/child.html"), "参照は子テンプレート内にあるべき");
}

/// ng-include経由で継承されたローカル変数がスコープ参照から除外されることを確認
#[rstest]
fn test_inherited_local_variable_not_in_scope_reference(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;

    // 親テンプレート
    let parent_html = r#"
<div ng-repeat="sheet in sheets">
  <ng-include src="'child.html'"></ng-include>
</div>
"#;
    let parent_uri = Url::parse("file:///parent.html").unwrap();
    analyzer.analyze_document(&parent_uri, parent_html);

    // 子テンプレート
    let child_html = r#"
<div ng-if="sheet.visible">
  <span>[[sheet.name]]</span>
</div>
"#;
    let child_uri = Url::parse("file:///child.html").unwrap();
    analyzer.analyze_document(&child_uri, child_html);

    // sheetはローカル変数なので、HtmlScopeReferenceとしては登録されないべき
    let scope_ref = index.find_html_scope_reference_at(&child_uri, 1, 12);
    assert!(scope_ref.is_none(), "継承されたローカル変数sheetはHtmlScopeReferenceに登録されないべき");

    // 代わりにHtmlLocalVariableReferenceとして登録されているべき
    let local_var_ref = index.find_html_local_variable_at(&child_uri, 1, 12);
    assert!(local_var_ref.is_some(), "継承されたローカル変数sheetはHtmlLocalVariableReferenceに登録されるべき");
}

/// 非ディレクティブ属性内のインターポレーションからスコープ参照を抽出
#[rstest]
fn test_interpolation_in_non_directive_attribute(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="UserController">
    <div class="{{dynamicClass}}">Content</div>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // class属性内のインターポレーションからスコープ参照が抽出されているか
    // `    <div class="{{dynamicClass}}"...` で dynamicClass は col 18 から開始
    let ref_opt = index.find_html_scope_reference_at(&uri, 2, 18);
    assert!(ref_opt.is_some(), "non-directive attribute interpolation reference should be found");
    assert_eq!(ref_opt.unwrap().property_path, "dynamicClass");
}

/// 非ディレクティブ属性内の複数インターポレーション
#[rstest]
fn test_multiple_interpolations_in_non_directive_attribute(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="UserController">
    <div title="Hello, {{name}}! You have {{count}} messages.">Content</div>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // title="Hello, {{name}}! You have {{count}} messages."
    // "name" は col 25 から開始
    let ref_name = index.find_html_scope_reference_at(&uri, 2, 25);
    assert!(ref_name.is_some(), "first interpolation reference should be found");
    assert_eq!(ref_name.unwrap().property_path, "name");

    // "count" は col 44 から開始
    let ref_count = index.find_html_scope_reference_at(&uri, 2, 44);
    assert!(ref_count.is_some(), "second interpolation reference should be found");
    assert_eq!(ref_count.unwrap().property_path, "count");
}

/// 非ディレクティブ属性内のインターポレーションでローカル変数参照
#[rstest]
fn test_local_variable_in_non_directive_attribute_interpolation(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="UserController">
    <div ng-repeat="item in items" class="item-{{item.id}}">
        {{item.name}}
    </div>
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // class="item-{{item.id}}" で item は col 49 から開始
    // ローカル変数としての参照が登録されているべき
    let local_ref = index.find_html_local_variable_at(&uri, 2, 49);
    assert!(local_ref.is_some(), "local variable in non-directive attribute interpolation should be found");
    assert_eq!(local_ref.unwrap().variable_name, "item");

    // スコープ参照としては登録されないべき
    let scope_ref = index.find_html_scope_reference_at(&uri, 2, 49);
    assert!(scope_ref.is_none(), "local variable should not be registered as scope reference");
}

/// 複数属性でのインターポレーション
#[rstest]
fn test_interpolation_in_multiple_attributes(ctx: TestContext) {
    let TestContext { analyzer, index, .. } = ctx;
    let uri = Url::parse("file:///test.html").unwrap();

    let html = r#"
<div ng-controller="UserController">
    <img src="{{imageUrl}}" alt="{{altText}}" title="{{tooltipText}}">
</div>
"#;
    analyzer.analyze_document(&uri, html);

    // src="{{imageUrl}}" で imageUrl は col 16 から開始
    let ref_src = index.find_html_scope_reference_at(&uri, 2, 16);
    assert!(ref_src.is_some(), "src interpolation should be found");
    assert_eq!(ref_src.unwrap().property_path, "imageUrl");

    // alt="{{altText}}" で altText は col 35 から開始
    let ref_alt = index.find_html_scope_reference_at(&uri, 2, 35);
    assert!(ref_alt.is_some(), "alt interpolation should be found");
    assert_eq!(ref_alt.unwrap().property_path, "altText");

    // title="{{tooltipText}}" で tooltipText は col 55 から開始
    let ref_title = index.find_html_scope_reference_at(&uri, 2, 55);
    assert!(ref_title.is_some(), "title interpolation should be found");
    assert_eq!(ref_title.unwrap().property_path, "tooltipText");
}