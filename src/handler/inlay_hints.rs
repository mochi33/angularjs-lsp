//! Inlay hints handler.
//!
//! AngularJS の DI 配列・controller as 構文では、識別子と「何のサービス /
//! コントローラーか」が syntactic に切り離されているため、ジャンプしないと
//! 対応関係が分からない。Inlay hints でこれを inline 表示する。
//!
//! 3 種類のヒント:
//!
//! 1. **DI rename hint** (JS):
//!    `['$scope', '$timeout', function(s, t)]` の `s` の右に `: $scope` を表示
//! 2. **controller as alias hint** (HTML):
//!    `<div ng-controller="MainCtrl as vm">{{ vm.foo }}` の `vm` の右に
//!    `: MainCtrl` を表示
//! 3. **`$ctrl` alias hint** (HTML):
//!    component template 内の `{{ $ctrl.bar }}` の `$ctrl` の右に
//!    `: <componentName>` を表示
//!
//! issue #66 参照。

use std::sync::Arc;

use dashmap::DashMap;
use tower_lsp::lsp_types::{
    InlayHint, InlayHintKind, InlayHintLabel, Position, Range, Url,
};
use tree_sitter::{Node, Parser};

use crate::index::Index;
use crate::util::{is_html_file, is_js_file};

pub struct InlayHintsHandler {
    index: Arc<Index>,
    documents: Arc<DashMap<Url, String>>,
}

impl InlayHintsHandler {
    pub fn new(index: Arc<Index>, documents: Arc<DashMap<Url, String>>) -> Self {
        Self { index, documents }
    }

    /// 指定範囲の inlay hints を計算する。
    ///
    /// `range` が `None` の場合はファイル全体から hints を集める。
    /// LSP の `textDocument/inlayHint` は範囲を絞ってくるため、ここで早期に
    /// フィルタを適用しておく。
    pub fn inlay_hints(&self, uri: &Url, range: Option<Range>) -> Option<Vec<InlayHint>> {
        let hints = if is_js_file(uri) {
            self.collect_js_hints(uri)
        } else if is_html_file(uri) {
            self.collect_html_hints(uri)
        } else {
            return None;
        };

        let filtered = match range {
            Some(r) => hints
                .into_iter()
                .filter(|h| position_in_range(h.position, r))
                .collect(),
            None => hints,
        };

        Some(filtered)
    }

    // ============================================================
    // JS: DI rename hint
    // ============================================================

    /// JS ファイルから DI rename hint を集める。
    ///
    /// 認識パターン: `['$scope', '$timeout', function(s, t) {...}]`
    /// param 名 `s` がサービス `$scope` に DI されていて、かつ param 名と
    /// サービス名が異なるときに `: $scope` を表示する。
    fn collect_js_hints(&self, uri: &Url) -> Vec<InlayHint> {
        let source = match self.documents.get(uri) {
            Some(doc) => doc.value().clone(),
            None => return Vec::new(),
        };

        let mut parser = Parser::new();
        if parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .is_err()
        {
            return Vec::new();
        }
        let tree = match parser.parse(&source, None) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let mut hints = Vec::new();
        collect_di_rename_hints(tree.root_node(), &source, &mut hints);
        hints
    }

    // ============================================================
    // HTML: controller as alias hint / $ctrl alias hint
    // ============================================================

    /// HTML ファイルから alias hint を集める。
    ///
    /// `vm.foo` の `vm` を `controller as alias` で resolve し、`: MainCtrl`
    /// を表示する。`$ctrl.foo` は component template の場合に component 名を
    /// 表示する。
    fn collect_html_hints(&self, uri: &Url) -> Vec<InlayHint> {
        let mut hints = Vec::new();

        let refs = self.index.html.get_html_scope_references(uri);
        for scope_ref in refs {
            let Some((alias, _rest)) = split_alias(&scope_ref.property_path) else {
                continue;
            };

            // `vm.foo` のような controller as alias を解決
            if let Some(controller_name) = self
                .index
                .resolve_controller_by_alias(uri, scope_ref.start_line, alias)
            {
                if alias == controller_name {
                    // alias 名 == controller 名 (例: `<div ng-controller="MainCtrl as MainCtrl">`)
                    // のときは hint を出さない (ノイズ)。
                    continue;
                }
                // `vm` の直後 (vm の end_col の位置 = start_col + alias.len()) に hint
                let alias_end_col = scope_ref.start_col + alias.chars().count() as u32;
                hints.push(make_hint(
                    Position {
                        line: scope_ref.start_line,
                        character: alias_end_col,
                    },
                    format!(": {}", controller_name),
                ));
                continue;
            }

            // `$ctrl.foo` (component template の controller_as) の場合は
            // component の controller 名を出す
            if let Some(controller_name) = self.resolve_component_alias(uri, alias) {
                if alias == controller_name {
                    continue;
                }
                let alias_end_col = scope_ref.start_col + alias.chars().count() as u32;
                hints.push(make_hint(
                    Position {
                        line: scope_ref.start_line,
                        character: alias_end_col,
                    },
                    format!(": {}", controller_name),
                ));
            }
        }

        hints
    }

    /// component template として登録された URI に対して、`alias` が
    /// `controller_as` と一致するときに controller 名を返す。
    ///
    /// component の controller_as が省略されている場合のデフォルトは `$ctrl`。
    fn resolve_component_alias(&self, uri: &Url, alias: &str) -> Option<String> {
        let binding = self.index.components.get_component_binding_for_template(uri)?;
        if binding.controller_as != alias {
            return None;
        }
        binding.controller_name
    }
}

/// `vm.foo` → `Some(("vm", "foo"))`、`vm` → `None`、`vm.user.name` →
/// `Some(("vm", "user.name"))`
fn split_alias(property_path: &str) -> Option<(&str, &str)> {
    let dot = property_path.find('.')?;
    let alias = &property_path[..dot];
    let rest = &property_path[dot + 1..];
    if alias.is_empty() || rest.is_empty() {
        return None;
    }
    Some((alias, rest))
}

/// hint の標準形を作るヘルパー。
fn make_hint(position: Position, label: String) -> InlayHint {
    InlayHint {
        position,
        label: InlayHintLabel::String(label),
        kind: Some(InlayHintKind::TYPE),
        text_edits: None,
        tooltip: None,
        padding_left: Some(false),
        padding_right: Some(false),
        data: None,
    }
}

/// `position` が `range` 内にあるか判定する。
/// LSP の半開区間に従い、`range.end` ちょうどは含まないとみなす。
fn position_in_range(pos: Position, range: Range) -> bool {
    if pos.line < range.start.line || pos.line > range.end.line {
        return false;
    }
    if pos.line == range.start.line && pos.character < range.start.character {
        return false;
    }
    if pos.line == range.end.line && pos.character > range.end.character {
        return false;
    }
    true
}

// ============================================================
// JS DI rename 抽出 (free functions)
// ============================================================

/// AST を再帰的に walk して DI rename hint を集める。
fn collect_di_rename_hints(node: Node, source: &str, hints: &mut Vec<InlayHint>) {
    if let Some((services, function_node)) = parse_di_array(node, source) {
        emit_di_hints_for_function(function_node, source, &services, hints);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_di_rename_hints(child, source, hints);
    }
}

/// `['s1', 's2', function(...){}]` 形式の配列から (services, function_node)
/// を取り出す。これに該当しなければ `None`。
fn parse_di_array<'a>(node: Node<'a>, source: &str) -> Option<(Vec<String>, Node<'a>)> {
    if node.kind() != "array" {
        return None;
    }
    let mut services: Vec<String> = Vec::new();
    let mut function_node: Option<Node<'a>> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "string" => services.push(extract_string_value(child, source)),
            "function_expression" | "arrow_function" => {
                function_node = Some(child);
            }
            _ => {}
        }
    }
    let function_node = function_node?;
    if services.is_empty() {
        return None;
    }
    Some((services, function_node))
}

/// 関数のパラメータと services を 1:1 で対応させて hint を作る。
fn emit_di_hints_for_function(
    function_node: Node,
    source: &str,
    services: &[String],
    hints: &mut Vec<InlayHint>,
) {
    let Some(params) = function_node.child_by_field_name("parameters") else {
        return;
    };

    let mut idx: usize = 0;
    let mut cursor = params.walk();
    for child in params.children(&mut cursor) {
        if child.kind() != "identifier" {
            continue;
        }
        if idx >= services.len() {
            break;
        }
        let param_name = node_text(child, source);
        let service = &services[idx];
        idx += 1;

        // param 名 == service 名 のときは情報量がないので hint を出さない
        if param_name == service.as_str() {
            continue;
        }

        // tree-sitter の end_position は半開区間で、参照識別子の直後を指す
        let end = child.end_position();
        hints.push(make_hint(
            Position {
                line: end.row as u32,
                character: end.column as u32,
            },
            format!(": {}", service),
        ));
    }
}

/// `string` ノードから引用符を外した値を取り出す。
fn extract_string_value(node: Node, source: &str) -> String {
    let raw = node_text(node, source);
    if raw.len() >= 2 {
        let bytes = raw.as_bytes();
        let first = bytes[0];
        let last = bytes[raw.len() - 1];
        if (first == b'"' || first == b'\'' || first == b'`') && first == last {
            return raw[1..raw.len() - 1].to_string();
        }
    }
    raw.to_string()
}

fn node_text<'a>(node: Node, source: &'a str) -> &'a str {
    &source[node.start_byte()..node.end_byte()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_js(source: &str) -> tree_sitter::Tree {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn split_alias_with_simple_path() {
        assert_eq!(split_alias("vm.foo"), Some(("vm", "foo")));
        assert_eq!(split_alias("$ctrl.bar"), Some(("$ctrl", "bar")));
        assert_eq!(split_alias("vm.user.name"), Some(("vm", "user.name")));
        assert_eq!(split_alias("vm"), None);
        assert_eq!(split_alias(""), None);
        assert_eq!(split_alias(".foo"), None);
        assert_eq!(split_alias("vm."), None);
    }

    #[test]
    fn position_in_range_basic() {
        let r = Range {
            start: Position { line: 1, character: 5 },
            end: Position { line: 3, character: 10 },
        };
        assert!(position_in_range(Position { line: 1, character: 5 }, r));
        assert!(position_in_range(Position { line: 2, character: 0 }, r));
        assert!(position_in_range(Position { line: 3, character: 10 }, r));
        assert!(!position_in_range(Position { line: 0, character: 100 }, r));
        assert!(!position_in_range(Position { line: 1, character: 4 }, r));
        assert!(!position_in_range(Position { line: 3, character: 11 }, r));
        assert!(!position_in_range(Position { line: 4, character: 0 }, r));
    }

    #[test]
    fn collect_di_rename_hints_array_form() {
        // ['$scope', '$timeout', function(s, t) {...}]
        let source = "\
angular.module('app').controller('Main', ['$scope', '$timeout', function(s, t) {\n\
    s.foo = 1;\n\
}]);\n";
        let tree = parse_js(source);
        let mut hints = Vec::new();
        collect_di_rename_hints(tree.root_node(), source, &mut hints);

        // 2 つの hint が出る (`s` の後ろと `t` の後ろ)
        assert_eq!(hints.len(), 2);
        // ラベルは `: $scope` / `: $timeout`
        let labels: Vec<String> = hints
            .iter()
            .map(|h| match &h.label {
                InlayHintLabel::String(s) => s.clone(),
                _ => String::new(),
            })
            .collect();
        assert!(labels.contains(&": $scope".to_string()));
        assert!(labels.contains(&": $timeout".to_string()));
    }

    #[test]
    fn collect_di_rename_hints_skips_when_param_matches_service() {
        // 暗黙 DI: param == service なので hint は出ない
        let source = "\
angular.module('app').controller('Main', ['$scope', function($scope) {\n\
    $scope.foo = 1;\n\
}]);\n";
        let tree = parse_js(source);
        let mut hints = Vec::new();
        collect_di_rename_hints(tree.root_node(), source, &mut hints);
        assert!(hints.is_empty(), "expected no hints, got {:?}", hints);
    }

    #[test]
    fn collect_di_rename_hints_partial_renaming() {
        // 1 つだけ rename されているケース
        let source = "\
angular.module('app').controller('Main', ['$scope', '$timeout', function($scope, t) {\n\
    $scope.foo = t;\n\
}]);\n";
        let tree = parse_js(source);
        let mut hints = Vec::new();
        collect_di_rename_hints(tree.root_node(), source, &mut hints);
        assert_eq!(hints.len(), 1);
        if let InlayHintLabel::String(label) = &hints[0].label {
            assert_eq!(label, ": $timeout");
        } else {
            panic!("expected String label");
        }
    }

    #[test]
    fn collect_di_rename_hints_position_after_param() {
        // `function(s, t)` の `s` の直後 (`,` の直前) の位置を期待
        let source = "var d = ['$scope', function(s) {}];\n";
        let tree = parse_js(source);
        let mut hints = Vec::new();
        collect_di_rename_hints(tree.root_node(), source, &mut hints);
        assert_eq!(hints.len(), 1);
        // `s` は line 0, col 28 にある (`var d = ['$scope', function(` の長さ)
        assert_eq!(hints[0].position.line, 0);
        assert_eq!(hints[0].position.character, 29);
    }

    #[test]
    fn collect_di_rename_hints_ignores_non_di_array() {
        // string 配列だけ (function なし) → DI 配列ではない
        let source = "var deps = ['$scope', '$timeout'];\n";
        let tree = parse_js(source);
        let mut hints = Vec::new();
        collect_di_rename_hints(tree.root_node(), source, &mut hints);
        assert!(hints.is_empty());
    }

    #[test]
    fn collect_di_rename_hints_nested_in_module_chain() {
        // ネストした module chain でも検出できる
        let source = "\
angular.module('app').config(['$routeProvider', function(rp) {\n\
    rp.when('/', {});\n\
}]);\n";
        let tree = parse_js(source);
        let mut hints = Vec::new();
        collect_di_rename_hints(tree.root_node(), source, &mut hints);
        assert_eq!(hints.len(), 1);
        if let InlayHintLabel::String(label) = &hints[0].label {
            assert_eq!(label, ": $routeProvider");
        } else {
            panic!("expected String label");
        }
    }

    // ============================================================
    // Integration tests for InlayHintsHandler::inlay_hints
    // ============================================================

    use crate::index::Index;
    use crate::model::{ComponentTemplateUrl, HtmlControllerScope, HtmlScopeReference};

    fn js_url() -> Url {
        Url::parse("file:///test.js").unwrap()
    }

    fn html_url() -> Url {
        Url::parse("file:///test.html").unwrap()
    }

    fn make_handler(
        index: Arc<Index>,
        documents: Arc<DashMap<Url, String>>,
    ) -> InlayHintsHandler {
        InlayHintsHandler::new(index, documents)
    }

    #[test]
    fn inlay_hints_di_rename_via_public_api() {
        let uri = js_url();
        let source = "\
angular.module('app').controller('Main', ['$scope', '$timeout', function(s, t) {\n\
    s.foo = 1;\n\
}]);\n";
        let documents = Arc::new(DashMap::new());
        documents.insert(uri.clone(), source.to_string());

        let handler = make_handler(Arc::new(Index::new()), documents);
        let hints = handler.inlay_hints(&uri, None).unwrap();
        assert_eq!(hints.len(), 2);
    }

    #[test]
    fn inlay_hints_controller_as_alias_via_public_api() {
        // <div ng-controller="MainCtrl as vm"> ... {{ vm.foo }} ... </div>
        let uri = html_url();
        let index = Arc::new(Index::new());

        // ng-controller スコープ (line 0..10, alias = "vm")
        index
            .controllers
            .add_html_controller_scope(HtmlControllerScope {
                controller_name: "MainCtrl".to_string(),
                alias: Some("vm".to_string()),
                uri: uri.clone(),
                start_line: 0,
                end_line: 10,
            });

        // {{ vm.foo }} をスコープ参照として登録 (line 5, col 3..9)
        index.html.add_html_scope_reference(HtmlScopeReference {
            property_path: "vm.foo".to_string(),
            uri: uri.clone(),
            start_line: 5,
            start_col: 3,
            end_line: 5,
            end_col: 9,
        });

        let handler = make_handler(index, Arc::new(DashMap::new()));
        let hints = handler.inlay_hints(&uri, None).unwrap();
        assert_eq!(hints.len(), 1);
        // `vm` の直後 (col 3 + 2 = 5) に `: MainCtrl`
        assert_eq!(hints[0].position.line, 5);
        assert_eq!(hints[0].position.character, 5);
        if let InlayHintLabel::String(label) = &hints[0].label {
            assert_eq!(label, ": MainCtrl");
        } else {
            panic!("expected String label");
        }
    }

    #[test]
    fn inlay_hints_dollar_ctrl_alias_via_public_api() {
        // component template に紐づく URI で `$ctrl.foo` 参照
        let uri = html_url();
        let index = Arc::new(Index::new());

        // component の templateUrl 紐づけ
        index
            .components
            .add_component_template_url(ComponentTemplateUrl {
                uri: Url::parse("file:///component.js").unwrap(),
                template_path: "/test.html".to_string(),
                line: 0,
                col: 0,
                controller_name: Some("UserCardController".to_string()),
                controller_as: "$ctrl".to_string(),
            });

        // {{ $ctrl.bar }}
        index.html.add_html_scope_reference(HtmlScopeReference {
            property_path: "$ctrl.bar".to_string(),
            uri: uri.clone(),
            start_line: 2,
            start_col: 4,
            end_line: 2,
            end_col: 13,
        });

        let handler = make_handler(index, Arc::new(DashMap::new()));
        let hints = handler.inlay_hints(&uri, None).unwrap();
        assert_eq!(hints.len(), 1);
        // `$ctrl` の直後 (col 4 + 5 = 9)
        assert_eq!(hints[0].position.line, 2);
        assert_eq!(hints[0].position.character, 9);
        if let InlayHintLabel::String(label) = &hints[0].label {
            assert_eq!(label, ": UserCardController");
        } else {
            panic!("expected String label");
        }
    }

    #[test]
    fn inlay_hints_range_filter_drops_hints_outside_visible_range() {
        let uri = js_url();
        let source = "\
var a = ['$scope', function(s) { s.foo = 1; }];\n\
var b = ['$timeout', function(t) { t.cancel(); }];\n";
        let documents = Arc::new(DashMap::new());
        documents.insert(uri.clone(), source.to_string());

        let handler = make_handler(Arc::new(Index::new()), documents);

        // 全体: 2 件
        let all = handler.inlay_hints(&uri, None).unwrap();
        assert_eq!(all.len(), 2);

        // line 0 のみに絞る → 1 件
        let only_first = handler
            .inlay_hints(
                &uri,
                Some(Range {
                    start: Position { line: 0, character: 0 },
                    end: Position { line: 0, character: 1000 },
                }),
            )
            .unwrap();
        assert_eq!(only_first.len(), 1);
        if let InlayHintLabel::String(label) = &only_first[0].label {
            assert_eq!(label, ": $scope");
        } else {
            panic!("expected String label");
        }
    }

    #[test]
    fn inlay_hints_returns_none_for_unknown_extension() {
        let uri = Url::parse("file:///test.css").unwrap();
        let handler = make_handler(Arc::new(Index::new()), Arc::new(DashMap::new()));
        assert!(handler.inlay_hints(&uri, None).is_none());
    }
}
