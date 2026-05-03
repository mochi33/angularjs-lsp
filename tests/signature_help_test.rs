///! signatureHelp ハンドラの統合テスト。
///!
///! AngularJS のシンボル (controller method, service method, factory method) の
///! 引数ヒントを LSP signatureHelp として返せるかを検証する。

use std::sync::Arc;
use tower_lsp::lsp_types::{ParameterLabel, Url};

use angularjs_lsp::analyzer::js::AngularJsAnalyzer;
use angularjs_lsp::handler::SignatureHelpHandler;
use angularjs_lsp::index::Index;

/// JS ソースを 1 ファイル解析した index と uri を返すヘルパー。
fn analyze_js_source(source: &str) -> (Arc<Index>, Url) {
    let index = Arc::new(Index::new());
    let analyzer = AngularJsAnalyzer::new(Arc::clone(&index));
    let uri = Url::parse("file:///test.js").unwrap();
    analyzer.analyze_document(&uri, source);
    (index, uri)
}

#[test]
fn signature_help_for_service_method_call_in_js() {
    // .service('UserService', function() { this.update = function(user, opts) {} })
    // を解析してから、別ファイル風に同じソース内の "UserService.update(<cursor>" の
    // 位置で signatureHelp を要求する。
    let source = r#"
angular.module('app', [])
.service('UserService', function() {
    this.update = function(user, opts) {};
});

function callIt() {
    UserService.update();
}
"#;
    let (index, uri) = analyze_js_source(source);

    // "UserService.update(" の `(` の直後 (開き括弧の中) にカーソルがある状態を作る
    // 8 行目: "    UserService.update();" — 0-indexed の 7
    // `(` の位置を特定して直後にカーソルを置く
    let line_text = "    UserService.update();";
    let paren_pos = line_text.find('(').unwrap() as u32 + 1;

    let handler = SignatureHelpHandler::new(index);
    let help = handler
        .signature_help(&uri, 7, paren_pos, source)
        .expect("signature help が返るべき (service method)");

    assert_eq!(help.signatures.len(), 1, "signature が 1 つ");
    let sig = &help.signatures[0];
    assert!(
        sig.label.contains("update"),
        "label にメソッド名が入る: {}",
        sig.label
    );
    let params = sig.parameters.as_ref().expect("parameters があるべき");
    let names: Vec<String> = params
        .iter()
        .map(|p| match &p.label {
            ParameterLabel::Simple(s) => s.clone(),
            ParameterLabel::LabelOffsets(_) => String::new(),
        })
        .collect();
    assert_eq!(names, vec!["user".to_string(), "opts".to_string()]);
    assert_eq!(help.active_parameter, Some(0));
}

#[test]
fn signature_help_for_service_method_active_parameter_after_comma() {
    // 第1引数の後 (カンマの後) にカーソルがある場合、active_parameter は 1。
    let source = r#"
angular.module('app', [])
.service('UserService', function() {
    this.update = function(user, opts) {};
});

function callIt() {
    UserService.update(currentUser, );
}
"#;
    let (index, uri) = analyze_js_source(source);

    // 8 行目 (0-indexed 7): "    UserService.update(currentUser, );"
    // カンマの後ろ・スペースの位置にカーソル
    let line_text = "    UserService.update(currentUser, );";
    let comma_pos = line_text.find(", ").unwrap() as u32 + 2; // ", " の後ろ

    let handler = SignatureHelpHandler::new(index);
    let help = handler
        .signature_help(&uri, 7, comma_pos, source)
        .expect("signature help が返るべき");

    assert_eq!(help.active_parameter, Some(1), "第2引数位置を示す");
    assert_eq!(help.signatures[0].active_parameter, Some(1));
}

#[test]
fn signature_help_returns_none_when_symbol_has_no_parameters() {
    // controller / service として登録されたシンボル (Component / Module 等) は
    // parameters: None のため、signatureHelp は None を返す。
    let source = r#"
angular.module('myApp', []);

function caller() {
    myApp();
}
"#;
    let (index, uri) = analyze_js_source(source);

    let line_text = "    myApp();";
    let paren_pos = line_text.find('(').unwrap() as u32 + 1;

    let handler = SignatureHelpHandler::new(index);
    let help = handler.signature_help(&uri, 4, paren_pos, source);

    assert!(
        help.is_none(),
        "引数情報のないシンボルでは signatureHelp は None"
    );
}

#[test]
fn signature_help_returns_none_when_no_call_context() {
    // 関数呼び出しの括弧外にカーソルがある場合は None。
    let source = r#"
angular.module('app', [])
.service('UserService', function() {
    this.update = function(user) {};
});
"#;
    let (index, uri) = analyze_js_source(source);

    // 1 行目 (空行) の col 0 にカーソル
    let handler = SignatureHelpHandler::new(index);
    let help = handler.signature_help(&uri, 0, 0, source);

    assert!(help.is_none(), "呼び出し外では signatureHelp は None");
}
