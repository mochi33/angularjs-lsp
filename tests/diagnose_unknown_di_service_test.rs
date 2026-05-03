//! `check_unknown_di_services` (issue #63) の統合テスト
//!
//! DI 配列内の string literal について:
//! 1. AngularJS 組み込みサービス allowlist と完全一致するなら静か
//! 2. workspace の Service / Factory / Provider / Value / Constant 定義に
//!    存在するなら静か
//! 3. どちらでもなければ警告。Levenshtein 距離 ≤ 2 の組み込みサービスがあれば
//!    "Did you mean '...'?" を message に含める
//!
//! workspace スキャン未完了時は静か (起動直後の false positive を抑止) も検証する。

use std::sync::Arc;

use tower_lsp::lsp_types::{DiagnosticSeverity, Url};

use angularjs_lsp::analyzer::js::AngularJsAnalyzer;
use angularjs_lsp::config::DiagnosticsConfig;
use angularjs_lsp::handler::DiagnosticsHandler;
use angularjs_lsp::index::Index;

fn js_uri() -> Url {
    Url::parse("file:///test.js").unwrap()
}

/// JS ソースを解析し、workspace scan 完了済み状態の Index を返す
fn analyze_and_mark_scanned(js_source: &str) -> Arc<Index> {
    let index = Arc::new(Index::new());
    let analyzer = AngularJsAnalyzer::new(Arc::clone(&index));
    analyzer.analyze_document(&js_uri(), js_source);
    index.mark_workspace_scanned();
    index
}

fn diagnose(index: Arc<Index>, uri: &Url) -> Vec<tower_lsp::lsp_types::Diagnostic> {
    let handler = DiagnosticsHandler::new(index, DiagnosticsConfig::default());
    handler.diagnose_js(uri)
}

#[test]
fn allowlist_builtin_service_is_silent() {
    // $scope / $http / $timeout は組み込みなので警告にならない
    let js = r#"
angular.module('app', [])
.controller('MyCtrl', ['$scope', '$http', '$timeout', function($scope, $http, $timeout) {
    $scope.go = function() {};
}]);
"#;
    let index = analyze_and_mark_scanned(js);
    let diagnostics = diagnose(index, &js_uri());

    // 未知サービス警告のみフィルタ
    let unknown_diags: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.message.starts_with("Unknown service"))
        .collect();
    assert!(
        unknown_diags.is_empty(),
        "組み込みサービスは警告にならないはず: {:?}",
        unknown_diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn known_workspace_service_is_silent() {
    // 同じファイル内に登録された UserService は警告にならない
    let js = r#"
angular.module('app', [])
.service('UserService', function() {
    this.fetch = function() {};
})
.controller('MyCtrl', ['$scope', 'UserService', function($scope, UserService) {
    $scope.u = UserService;
}]);
"#;
    let index = analyze_and_mark_scanned(js);
    let diagnostics = diagnose(index, &js_uri());

    let unknown_diags: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.message.starts_with("Unknown service"))
        .collect();
    assert!(
        unknown_diags.is_empty(),
        "登録済み workspace サービスは警告にならないはず: {:?}",
        unknown_diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn known_workspace_factory_is_silent() {
    let js = r#"
angular.module('app', [])
.factory('AuthFactory', function() { return {}; })
.controller('MyCtrl', ['AuthFactory', function(AuthFactory) {}]);
"#;
    let index = analyze_and_mark_scanned(js);
    let diagnostics = diagnose(index, &js_uri());

    let unknown_diags: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.message.starts_with("Unknown service"))
        .collect();
    assert!(unknown_diags.is_empty());
}

#[test]
fn known_workspace_value_and_constant_are_silent() {
    let js = r#"
angular.module('app', [])
.value('appConfig', { foo: 1 })
.constant('API_URL', '/api')
.controller('MyCtrl', ['appConfig', 'API_URL', function(c, u) {}]);
"#;
    let index = analyze_and_mark_scanned(js);
    let diagnostics = diagnose(index, &js_uri());

    let unknown_diags: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.message.starts_with("Unknown service"))
        .collect();
    assert!(unknown_diags.is_empty(), "Value / Constant は警告にならないはず");
}

#[test]
fn typo_of_builtin_service_is_warned_with_suggestion() {
    // $tiemout (typo) は $timeout と Levenshtein 距離 2 → "Did you mean '$timeout'?"
    let js = r#"
angular.module('app', [])
.controller('MyCtrl', ['$scope', '$tiemout', function($scope, t) {
    t(function() {}, 100);
}]);
"#;
    let index = analyze_and_mark_scanned(js);
    let diagnostics = diagnose(index, &js_uri());

    let unknown_diags: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.message.starts_with("Unknown service"))
        .collect();

    assert_eq!(
        unknown_diags.len(),
        1,
        "$tiemout 1件のみ警告のはず。実際: {:?}",
        unknown_diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    let msg = &unknown_diags[0].message;
    assert!(
        msg.contains("$tiemout"),
        "メッセージにユーザの書いた名前を含めるべき: {}",
        msg
    );
    assert!(
        msg.contains("$timeout"),
        "Levenshtein 候補 $timeout を含めるべき: {}",
        msg
    );
    assert!(
        msg.contains("Did you mean"),
        "メッセージに 'Did you mean' を含めるべき: {}",
        msg
    );
    assert_eq!(
        unknown_diags[0].severity,
        Some(DiagnosticSeverity::WARNING)
    );
}

#[test]
fn completely_unknown_service_is_warned_without_suggestion() {
    // どの組み込みサービスとも遠い名前なら "Did you mean" は付かない
    let js = r#"
angular.module('app', [])
.controller('MyCtrl', ['CompletelyUnknownXyz', function(x) {}]);
"#;
    let index = analyze_and_mark_scanned(js);
    let diagnostics = diagnose(index, &js_uri());

    let unknown_diags: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.message.starts_with("Unknown service"))
        .collect();

    assert_eq!(unknown_diags.len(), 1);
    let msg = &unknown_diags[0].message;
    assert!(msg.contains("CompletelyUnknownXyz"));
    assert!(
        !msg.contains("Did you mean"),
        "遠すぎる名前には候補を出さない: {}",
        msg
    );
}

#[test]
fn silent_until_workspace_scan_completed() {
    // mark_workspace_scanned を呼ばない → 警告は一切出ない (false positive 抑止)
    let js = r#"
angular.module('app', [])
.controller('MyCtrl', ['$tiemout', 'Unknown', function(a, b) {}]);
"#;
    let index = Arc::new(Index::new());
    let analyzer = AngularJsAnalyzer::new(Arc::clone(&index));
    analyzer.analyze_document(&js_uri(), js);
    // ★ ここで mark_workspace_scanned() を呼ばない

    let diagnostics = diagnose(index, &js_uri());

    let unknown_diags: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.message.starts_with("Unknown service"))
        .collect();
    assert!(
        unknown_diags.is_empty(),
        "scan 未完了の状態では警告は出ないはず: {:?}",
        unknown_diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn inject_pattern_typo_is_warned() {
    // `MyCtrl.$inject = [...]` パターンでも typo が検出されるか
    let js = r#"
function MyCtrl(s, t) {}
MyCtrl.$inject = ['$scope', '$tiemout'];
angular.module('app', []).controller('MyCtrl', MyCtrl);
"#;
    let index = analyze_and_mark_scanned(js);
    let diagnostics = diagnose(index, &js_uri());

    let unknown_diags: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.message.starts_with("Unknown service"))
        .collect();
    assert!(
        !unknown_diags.is_empty(),
        "$inject 経由でも typo を検出すべき"
    );
    assert!(unknown_diags.iter().any(|d| d.message.contains("$timeout")));
}

#[test]
fn severity_off_disables_diagnostic() {
    let js = r#"
angular.module('app', [])
.controller('MyCtrl', ['$tiemout', function(t) {}]);
"#;
    let index = analyze_and_mark_scanned(js);

    let config = DiagnosticsConfig {
        unknown_di_service_severity: "off".to_string(),
        ..Default::default()
    };

    let handler = DiagnosticsHandler::new(index, config);
    let diagnostics = handler.diagnose_js(&js_uri());
    let unknown_diags: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.message.starts_with("Unknown service"))
        .collect();
    assert!(
        unknown_diags.is_empty(),
        "severity=off で診断は無効化されるはず"
    );
}

#[test]
fn severity_error_changes_diagnostic_severity() {
    let js = r#"
angular.module('app', [])
.controller('MyCtrl', ['CompletelyUnknownAbc', function(x) {}]);
"#;
    let index = analyze_and_mark_scanned(js);

    let config = DiagnosticsConfig {
        unknown_di_service_severity: "error".to_string(),
        ..Default::default()
    };

    let handler = DiagnosticsHandler::new(index, config);
    let diagnostics = handler.diagnose_js(&js_uri());

    let unknown_diags: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.message.starts_with("Unknown service"))
        .collect();
    assert_eq!(unknown_diags.len(), 1);
    assert_eq!(unknown_diags[0].severity, Some(DiagnosticSeverity::ERROR));
}

#[test]
fn diagnostic_range_points_at_string_literal() {
    let js = "angular.module('app', []).controller('C', ['$tiemout', function(t){}]);";
    let index = analyze_and_mark_scanned(js);
    let diagnostics = diagnose(index, &js_uri());

    let unknown_diags: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.message.starts_with("Unknown service"))
        .collect();
    assert_eq!(unknown_diags.len(), 1);

    // 範囲は '$tiemout' (クォート込み) を指しているはず
    let range = unknown_diags[0].range;
    assert_eq!(range.start.line, 0);
    assert_eq!(range.end.line, 0);
    let snippet = &js[range.start.character as usize..range.end.character as usize];
    assert_eq!(snippet, "'$tiemout'");
}

#[test]
fn typo_of_user_service_is_still_warned_when_no_close_builtin() {
    // ユーザ定義 UserService に対する typo `UserServce` は組み込み候補が無いので
    // (Levenshtein で組み込みサービスに近いものが無い) "Unknown service" になる。
    // 候補提案は組み込みのみが対象なので "Did you mean" は付かない。
    let js = r#"
angular.module('app', [])
.service('UserService', function() {})
.controller('MyCtrl', ['UserServce', function(u) {}]);
"#;
    let index = analyze_and_mark_scanned(js);
    let diagnostics = diagnose(index, &js_uri());

    let unknown_diags: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.message.starts_with("Unknown service"))
        .collect();
    assert_eq!(unknown_diags.len(), 1);
    let msg = &unknown_diags[0].message;
    assert!(msg.contains("UserServce"));
}
