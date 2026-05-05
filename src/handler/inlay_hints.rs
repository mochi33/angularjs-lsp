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

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use dashmap::DashMap;
use tower_lsp::lsp_types::{
    InlayHint, InlayHintKind, InlayHintLabel, Position, Range, Url,
};
use tree_sitter::{Node, Parser, Tree};

use crate::index::Index;
use crate::util::{is_html_file, is_js_file};

/// JS の tree-sitter Tree を URI ごとにキャッシュする。
///
/// LSP の `textDocument/inlayHint` はクライアントのスクロール毎に呼ばれるが、
/// その間にソースが変わっていなければ Tree を使い回せる。
/// `source_hash` を毎回比較し、ソースが変化したときだけ再パースする。
pub struct CachedJsTree {
    source_hash: u64,
    tree: Tree,
}

/// `Backend` から共有する JS Tree キャッシュ。`Backend::new` で生成して、
/// `InlayHintsHandler::new` に渡す。
pub type JsTreeCache = DashMap<Url, CachedJsTree>;

pub fn new_js_tree_cache() -> Arc<JsTreeCache> {
    Arc::new(DashMap::new())
}

pub struct InlayHintsHandler {
    index: Arc<Index>,
    documents: Arc<DashMap<Url, String>>,
    js_tree_cache: Arc<JsTreeCache>,
}

impl InlayHintsHandler {
    pub fn new(
        index: Arc<Index>,
        documents: Arc<DashMap<Url, String>>,
        js_tree_cache: Arc<JsTreeCache>,
    ) -> Self {
        Self {
            index,
            documents,
            js_tree_cache,
        }
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
    ///
    /// 静的代入パターン (`MyCtrl.$inject = ['$scope']; function MyCtrl(s) {}`)
    /// は array 注釈と違って関数定義と DI が syntactic に離れているため
    /// 別 issue として future work。array 形式のみ対応する。
    ///
    /// パフォーマンス: ソースの hash を URI ごとに保存し、変化していない
    /// 場合は前回パースした Tree を再利用する (フルパース回避)。
    fn collect_js_hints(&self, uri: &Url) -> Vec<InlayHint> {
        let source = match self.documents.get(uri) {
            Some(doc) => doc.value().clone(),
            None => return Vec::new(),
        };

        let tree = match self.get_or_parse_js_tree(uri, &source) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let mut hints = Vec::new();
        collect_di_rename_hints(tree.root_node(), &source, &mut hints);
        hints
    }

    /// `js_tree_cache` から URI 対応の Tree を取得 (hash 一致時)、または
    /// 再パースしてキャッシュ更新する。`Tree::clone` は内部 Arc 参照のため
    /// 安価。
    fn get_or_parse_js_tree(&self, uri: &Url, source: &str) -> Option<Tree> {
        let new_hash = hash_source(source);

        if let Some(entry) = self.js_tree_cache.get(uri) {
            if entry.source_hash == new_hash {
                return Some(entry.tree.clone());
            }
        }

        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .ok()?;
        let tree = parser.parse(source, None)?;

        self.js_tree_cache.insert(
            uri.clone(),
            CachedJsTree {
                source_hash: new_hash,
                tree: tree.clone(),
            },
        );

        Some(tree)
    }

    // ============================================================
    // HTML: controller as alias hint / $ctrl alias hint
    // ============================================================

    /// HTML ファイルから alias hint を集める。
    ///
    /// `vm.foo` の `vm` を `controller as alias` で resolve し、`: MainCtrl`
    /// を表示する。`$ctrl.foo` は component template の場合に component 名を
    /// 表示する。
    ///
    /// 注: `Index::resolve_controller_by_alias` は内部で ng-controller →
    /// component の順にフォールバックするが、component 側の解決は URI に
    /// 対して 1 回計算すれば足りる (ref ごとに変わらない)。ここでは
    /// `controllers` (line 依存) を ref ごとに、component binding を URI ごとに
    /// 1 度だけ取得して per-ref のオーバーヘッドを抑える。
    fn collect_html_hints(&self, uri: &Url) -> Vec<InlayHint> {
        let mut hints = Vec::new();

        // URI に対して 1 回だけ component binding を取得 (ref ごとには変わらない)
        let component_binding = self
            .index
            .components
            .get_component_binding_for_template(uri);

        let refs = self.index.html.get_html_scope_references(uri);
        for scope_ref in refs {
            let Some((alias, _rest)) = split_alias(&scope_ref.property_path) else {
                continue;
            };

            // ng-controller as alias (line に依存) → component controller_as
            // (URI 単位、hoist 済みの binding を参照) の順に解決
            let controller_name = self
                .index
                .controllers
                .resolve_controller_by_alias(uri, scope_ref.start_line, alias)
                .or_else(|| {
                    component_binding.as_ref().and_then(|b| {
                        if b.controller_as == alias {
                            b.controller_name.clone()
                        } else {
                            None
                        }
                    })
                });

            let Some(controller_name) = controller_name else {
                continue;
            };

            if alias == controller_name {
                // alias 名 == controller 名 (例: `<div ng-controller="MainCtrl as MainCtrl">`)
                // のときは hint を出さない (ノイズ)。
                continue;
            }

            // `vm` の直後 = start_col + alias の UTF-16 code unit 数
            // (LSP の position は UTF-16 単位なので、char count ではなく
            //  utf16 unit count を使う必要がある — `vm` 等 ASCII では同値)
            let alias_end_col = scope_ref.start_col + alias.encode_utf16().count() as u32;
            hints.push(make_hint(
                Position {
                    line: scope_ref.start_line,
                    character: alias_end_col,
                },
                format!(": {}", controller_name),
            ));
        }

        hints
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
///
/// `kind` は全 hint で `TYPE` を採用 (clangd / rust-analyzer の慣習に倣い、
/// `: <name>` 形式のラベルは `TYPE` 扱い)。DI rename は意味的には `PARAMETER`
/// にも近いが、見た目の一貫性を優先している。
/// ラベル先頭にスペースを埋め込んで `padding_left/right: false` としており、
/// クライアント側のパディング差異に左右されにくい。
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
///
/// inlay hint の filter 用途では「可視範囲の末尾ちょうどに位置する hint」も
/// 表示したいので、ここでは閉区間 (両端含む) として扱う。LSP の Range は
/// 一般に半開だが、この関数は client から渡された可視 range に対する
/// 「hint を表示すべきか」の判定でしかないため UX を優先している。
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

        // tree-sitter の end_position は半開区間で、参照識別子の直後を指す。
        // ただし column は **バイト単位**。LSP の Position.character は
        // UTF-16 code unit 単位なので、変換が必要 (param 名手前に非 ASCII が
        // ある場合に位置がズレるのを防ぐ)。
        let end = child.end_position();
        let utf16_col = byte_offset_to_utf16_col(source, child.end_byte()) as u32;
        hints.push(make_hint(
            Position {
                line: end.row as u32,
                character: utf16_col,
            },
            format!(": {}", service),
        ));
    }
}

/// JS ソースのキャッシュキー用ハッシュ。`DefaultHasher` はプロセス内で
/// 決定的なので、同一プロセスで「ソースが変わったか」の判定に使える。
fn hash_source(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// ソース全体の絶対バイト offset から「その行内での UTF-16 code unit 数」を
/// 計算する。LSP は UTF-16 列を要求するため、tree-sitter のバイト列との
/// 変換にこのヘルパーを使う。
fn byte_offset_to_utf16_col(source: &str, byte_offset: usize) -> usize {
    let end = byte_offset.min(source.len());
    let prefix = &source[..end];
    let line_start = prefix.rfind('\n').map(|p| p + 1).unwrap_or(0);
    source[line_start..end]
        .chars()
        .map(|c| c.len_utf16())
        .sum()
}

/// `string` ノードから引用符を外した値を取り出す。
///
/// JavaScript の string literal は必ず ASCII の引用符 (`"` / `'` / `` ` ``)
/// で開閉されるため、最初/最後のバイトは常に 1 バイト。`raw[1..raw.len()-1]`
/// のバイトスライス境界は確実に文字境界に乗るのでこの実装で安全。
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
        InlayHintsHandler::new(index, documents, new_js_tree_cache())
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

        // ラベル中身まで公開 API 経由で確認 (unit test と並んで二重防御)
        let labels: Vec<String> = hints
            .iter()
            .filter_map(|h| match &h.label {
                InlayHintLabel::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        assert!(
            labels.contains(&": $scope".to_string()),
            "expected `: $scope` in {:?}",
            labels
        );
        assert!(
            labels.contains(&": $timeout".to_string()),
            "expected `: $timeout` in {:?}",
            labels
        );
    }

    #[test]
    fn byte_offset_to_utf16_col_handles_non_ascii_prefix() {
        // 1 行目: `var s = '🎉'; function f(x) {}` の `x` の位置
        // 🎉 は UTF-8 で 4 バイト、UTF-16 で 2 code unit
        let source = "var s = '🎉'; function f(x) {}\n";
        // `x` のバイト位置を探す
        let byte_idx = source.find("x").unwrap();
        let utf16_col = byte_offset_to_utf16_col(source, byte_idx);
        // 期待値: source[..byte_idx] を UTF-16 で数えた値
        let expected: usize = source[..byte_idx].chars().map(|c| c.len_utf16()).sum();
        assert_eq!(utf16_col, expected);
        // 🎉 を含むため byte 列より UTF-16 列のほうが小さい (バイト列 - 2)
        assert!(utf16_col < byte_idx);
    }

    #[test]
    fn byte_offset_to_utf16_col_resets_at_newline() {
        // 改行を跨ぐ場合は「その行の頭から」の UTF-16 数を返す
        let source = "first line\nsecond line";
        let byte_idx = source.find("second").unwrap();
        let utf16_col = byte_offset_to_utf16_col(source, byte_idx);
        assert_eq!(utf16_col, 0);
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

    #[test]
    fn js_tree_cache_reuses_tree_when_source_unchanged() {
        // 同じソースで 2 回呼ぶと、2 回目は cache から Tree が再利用され、
        // cache のサイズは 1 のまま、source_hash も等しい。
        let uri = js_url();
        let source = "var d = ['$scope', function(s) { s.foo = 1; }];\n";
        let documents = Arc::new(DashMap::new());
        documents.insert(uri.clone(), source.to_string());
        let cache = new_js_tree_cache();

        let handler = InlayHintsHandler::new(
            Arc::new(Index::new()),
            documents.clone(),
            Arc::clone(&cache),
        );

        let first = handler.inlay_hints(&uri, None).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(cache.len(), 1);
        let hash_after_first = cache.get(&uri).unwrap().source_hash;

        // 2 回目: 同じソース → 同じ hash で再利用
        let second = handler.inlay_hints(&uri, None).unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&uri).unwrap().source_hash, hash_after_first);
    }

    #[test]
    fn js_tree_cache_invalidates_on_source_change() {
        // ソースが変わると hash が変わり、再パース + cache 更新される。
        let uri = js_url();
        let documents = Arc::new(DashMap::new());
        documents.insert(
            uri.clone(),
            "var d = ['$scope', function(s) { s.foo = 1; }];\n".to_string(),
        );
        let cache = new_js_tree_cache();
        let handler = InlayHintsHandler::new(
            Arc::new(Index::new()),
            documents.clone(),
            Arc::clone(&cache),
        );

        let first = handler.inlay_hints(&uri, None).unwrap();
        assert_eq!(first.len(), 1);
        let hash1 = cache.get(&uri).unwrap().source_hash;

        // ドキュメントを差し替え
        documents.insert(
            uri.clone(),
            "var d = ['$timeout', function(t) { t.cancel(); }];\n".to_string(),
        );

        let second = handler.inlay_hints(&uri, None).unwrap();
        assert_eq!(second.len(), 1);
        let hash2 = cache.get(&uri).unwrap().source_hash;
        assert_ne!(hash1, hash2, "hash should change after source edit");

        if let InlayHintLabel::String(label) = &second[0].label {
            assert_eq!(label, ": $timeout");
        } else {
            panic!("expected String label");
        }
    }
}
