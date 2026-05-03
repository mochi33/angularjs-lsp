//! Issue #69: 未定義 \$scope プロパティ → controller に追加するクイックフィックス

use std::collections::HashMap;
use std::sync::Arc;

use tower_lsp::lsp_types::{
    CodeActionContext, CodeActionOrCommand, CodeActionParams, PartialResultParams, Position,
    Range, TextDocumentIdentifier, Url, WorkDoneProgressParams,
};

use angularjs_lsp::analyzer::html::HtmlAngularJsAnalyzer;
use angularjs_lsp::analyzer::js::AngularJsAnalyzer;
use angularjs_lsp::config::DiagnosticsConfig;
use angularjs_lsp::handler::{CodeActionHandler, DiagnosticsHandler};
use angularjs_lsp::index::Index;

/// JS と HTML を解析して Index を返すヘルパー (テスト用 URL 固定)。
fn analyze(
    js_source: &str,
    html_source: &str,
) -> (Arc<Index>, Url, Url) {
    let index = Arc::new(Index::new());
    let js_analyzer = Arc::new(AngularJsAnalyzer::new(index.clone()));
    let html_analyzer = HtmlAngularJsAnalyzer::new(index.clone(), js_analyzer.clone());

    let js_uri = Url::parse("file:///test.js").unwrap();
    js_analyzer.analyze_document(&js_uri, js_source);

    let html_uri = Url::parse("file:///test.html").unwrap();
    html_analyzer.analyze_document(&html_uri, html_source);

    (index, js_uri, html_uri)
}

/// 指定 URI に対して診断 → code action を求めるヘルパー。
/// `range_at_first_diag` が true の場合は params.range を最初の診断範囲に合わせる。
fn run_code_action(
    index: &Arc<Index>,
    html_uri: &Url,
    js_uri: &Url,
    js_source: &str,
) -> Vec<CodeActionOrCommand> {
    let diagnostics = DiagnosticsHandler::new(Arc::clone(index), DiagnosticsConfig::default())
        .diagnose_html(html_uri);

    // 最初の診断範囲を range として使う
    let range = diagnostics
        .first()
        .map(|d| d.range)
        .unwrap_or(Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 0,
            },
        });

    let params = CodeActionParams {
        text_document: TextDocumentIdentifier {
            uri: html_uri.clone(),
        },
        range,
        context: CodeActionContext {
            diagnostics,
            only: None,
            trigger_kind: None,
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let mut sources = HashMap::new();
    sources.insert(js_uri.clone(), js_source.to_string());

    CodeActionHandler::new(Arc::clone(index))
        .code_action(params, &sources)
        .unwrap_or_default()
}

/// `controller as vm` 構文で未定義 `vm.foo` 参照に対し `this.foo = null;` を
/// 挿入する code action が生成される。
#[test]
fn test_code_action_inserts_this_property_for_controller_as() {
    let js = r#"
angular.module('app', []).controller('MainCtrl', ['$scope', function($scope) {
    $scope.bar = 1;
}]);
"#;
    let html = r#"
<div ng-controller="MainCtrl as vm">
    {{ vm.foo }}
</div>
"#;
    let (index, js_uri, html_uri) = analyze(js, html);

    let actions = run_code_action(&index, &html_uri, &js_uri, js);

    // 少なくとも 1 件は出るはず
    assert!(
        !actions.is_empty(),
        "未定義 vm.foo に対して code action が出るべき (actions: {:?})",
        actions
    );

    // controller as → this.foo を優先
    let titles: Vec<String> = actions
        .iter()
        .filter_map(|a| match a {
            CodeActionOrCommand::CodeAction(ca) => Some(ca.title.clone()),
            _ => None,
        })
        .collect();
    assert!(
        titles.iter().any(|t| t.contains("this.foo")),
        "controller as の場合は 'this.foo' を提案するべき (titles: {:?})",
        titles
    );
    assert!(
        titles.iter().any(|t| t.contains("MainCtrl")),
        "controller 名 'MainCtrl' が title に含まれるべき (titles: {:?})",
        titles
    );

    // edit が controller の JS body に対するものになっていること
    let action = actions
        .iter()
        .find_map(|a| match a {
            CodeActionOrCommand::CodeAction(ca) if ca.title.contains("this.foo") => Some(ca),
            _ => None,
        })
        .expect("this.foo を含む action が必要");
    let edit = action.edit.as_ref().expect("WorkspaceEdit が必要");
    let changes = edit.changes.as_ref().expect("changes が必要");
    let edits = changes
        .get(&js_uri)
        .expect("controller JS の URI に対する TextEdit があるべき");
    assert_eq!(edits.len(), 1, "1 件の挿入");
    let new_text = &edits[0].new_text;
    assert!(
        new_text.contains("this.foo = null;"),
        "挿入テキストに 'this.foo = null;' が含まれるべき (text: {:?})",
        new_text
    );
}

/// `$scope.foo` 形式 (alias なし、`{{ foo }}`) の未定義参照では
/// `$scope.foo = null;` 挿入の code action が出る。
#[test]
fn test_code_action_inserts_scope_property_for_dollar_scope_form() {
    let js = r#"
angular.module('app', []).controller('UserCtrl', ['$scope', function($scope) {
    $scope.bar = 1;
}]);
"#;
    let html = r#"
<div ng-controller="UserCtrl">
    {{ foo }}
</div>
"#;
    let (index, js_uri, html_uri) = analyze(js, html);

    let actions = run_code_action(&index, &html_uri, &js_uri, js);
    assert!(
        !actions.is_empty(),
        "未定義 foo に対して code action が出るべき"
    );

    let titles: Vec<String> = actions
        .iter()
        .filter_map(|a| match a {
            CodeActionOrCommand::CodeAction(ca) => Some(ca.title.clone()),
            _ => None,
        })
        .collect();
    assert!(
        titles.iter().any(|t| t.contains("$scope.foo")),
        "alias なし参照では '$scope.foo' を提案するべき (titles: {:?})",
        titles
    );

    // 挿入テキストの確認
    let action = actions
        .iter()
        .find_map(|a| match a {
            CodeActionOrCommand::CodeAction(ca) if ca.title.contains("$scope.foo") => Some(ca),
            _ => None,
        })
        .expect("$scope.foo を含む action が必要");
    let edit = action.edit.as_ref().expect("WorkspaceEdit が必要");
    let changes = edit.changes.as_ref().expect("changes が必要");
    let edits = changes
        .get(&js_uri)
        .expect("controller JS の URI に対する TextEdit があるべき");
    let new_text = &edits[0].new_text;
    assert!(
        new_text.contains("$scope.foo = null;"),
        "挿入テキストに '$scope.foo = null;' が含まれるべき (text: {:?})",
        new_text
    );
}

/// 候補となる ControllerScope (= `$scope` 注入された JS controller) が無い場合、
/// code action は提示されない。
#[test]
fn test_code_action_returns_none_when_no_controller_scope() {
    // controller 自体は定義されているが $scope 注入が無いため ControllerScope は登録されない
    let js = r#"
angular.module('app', []).controller('NoScopeCtrl', function() {
    this.bar = 1;
});
"#;
    let html = r#"
<div ng-controller="NoScopeCtrl as vm">
    {{ vm.foo }}
</div>
"#;
    let (index, js_uri, html_uri) = analyze(js, html);

    let actions = run_code_action(&index, &html_uri, &js_uri, js);
    // ControllerScope (＝ \$scope DI) が無い → 挿入位置が決まらない → code action 0 件
    assert!(
        actions.is_empty(),
        "ControllerScope が無い場合は code action を出さない (actions: {:?})",
        actions
    );
}

/// 別の診断範囲 (request の range と重ならない) には code action を出さない。
#[test]
fn test_code_action_filters_by_range() {
    let js = r#"
angular.module('app', []).controller('MainCtrl', ['$scope', function($scope) {
    $scope.bar = 1;
}]);
"#;
    let html = r#"
<div ng-controller="MainCtrl as vm">
    {{ vm.foo }}
</div>
"#;
    let (index, js_uri, html_uri) = analyze(js, html);

    let diagnostics = DiagnosticsHandler::new(Arc::clone(&index), DiagnosticsConfig::default())
        .diagnose_html(&html_uri);
    assert!(
        !diagnostics.is_empty(),
        "前提として未定義診断が出ること"
    );

    // 診断範囲とは関係ない別の場所を range として渡す
    let unrelated_range = Range {
        start: Position {
            line: 100,
            character: 0,
        },
        end: Position {
            line: 100,
            character: 1,
        },
    };

    let params = CodeActionParams {
        text_document: TextDocumentIdentifier {
            uri: html_uri.clone(),
        },
        range: unrelated_range,
        context: CodeActionContext {
            diagnostics,
            only: None,
            trigger_kind: None,
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let mut sources = HashMap::new();
    sources.insert(js_uri.clone(), js.to_string());

    let result = CodeActionHandler::new(Arc::clone(&index)).code_action(params, &sources);
    assert!(
        result.is_none() || result.unwrap().is_empty(),
        "params.range と重ならない診断には code action を出さない"
    );
}
