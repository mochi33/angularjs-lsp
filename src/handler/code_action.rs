//! Code action ハンドラ。
//!
//! 現状サポート:
//! - **未定義 \$scope プロパティのクイックフィックス** (Issue #69)
//!   `{{ vm.foo }}` の `foo` が controller に未定義というケースで、controller 関数
//!   本体の先頭に `this.foo = null;` (controller as) または `$scope.foo = null;`
//!   を挿入する CodeAction を提示する。
//!
//! `diagnostics.rs` 側で警告に
//! [`UNDEFINED_SCOPE_PROPERTY_CODE`](super::diagnostics::UNDEFINED_SCOPE_PROPERTY_CODE)
//! を `code` フィールドにセットしてあり、本ハンドラはそれをキーに該当診断を識別する。

use std::collections::HashMap;
use std::sync::Arc;

use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams, CodeActionResponse,
    NumberOrString, Position, Range, TextEdit, Url, WorkspaceEdit,
};

use crate::handler::diagnostics::UNDEFINED_SCOPE_PROPERTY_CODE;
use crate::index::Index;
use crate::model::ControllerScope;
use crate::util::is_html_file;

/// Code action ハンドラ。
pub struct CodeActionHandler {
    index: Arc<Index>,
}

impl CodeActionHandler {
    pub fn new(index: Arc<Index>) -> Self {
        Self { index }
    }

    /// `textDocument/codeAction` のメインエントリ。
    ///
    /// `sources` は controller 側 JS の最新ソース (open buffer から得られる場合) を
    /// 渡すと、インデント推定に使われる。`None` でも動作する (デフォルト 4 spaces)。
    pub fn code_action(
        &self,
        params: CodeActionParams,
        sources: &HashMap<Url, String>,
    ) -> Option<CodeActionResponse> {
        let html_uri = params.text_document.uri.clone();
        if !is_html_file(&html_uri) {
            return None;
        }

        let mut actions: Vec<CodeActionOrCommand> = Vec::new();

        for diagnostic in &params.context.diagnostics {
            // 自分が出した未定義 \$scope プロパティ診断のみを処理する
            if !is_undefined_scope_property_diagnostic(diagnostic) {
                continue;
            }
            // params.range と diagnostic 範囲が重なっていない場合はスキップ
            if !ranges_overlap(&params.range, &diagnostic.range) {
                continue;
            }

            actions.extend(self.actions_for_diagnostic(&html_uri, diagnostic, sources));
        }

        if actions.is_empty() {
            None
        } else {
            Some(actions)
        }
    }

    /// 単一の診断に対応する CodeAction 群を生成する。
    fn actions_for_diagnostic(
        &self,
        html_uri: &Url,
        diagnostic: &tower_lsp::lsp_types::Diagnostic,
        sources: &HashMap<Url, String>,
    ) -> Vec<CodeActionOrCommand> {
        let mut actions: Vec<CodeActionOrCommand> = Vec::new();

        let line = diagnostic.range.start.line;
        let col = diagnostic.range.start.character;

        // 該当位置の HTML scope reference を引く
        let html_ref = match self
            .index
            .html
            .find_html_scope_reference_at(html_uri, line, col)
        {
            Some(r) => r,
            None => return actions,
        };

        // property_path を "alias" と "property" に分解する
        // 例: "vm.foo" -> ("vm", "foo"); "foo" -> (None, "foo")
        let (alias, property) = split_alias_and_property(&html_ref.property_path);

        // 解決すべき controller を決定
        // - alias がある場合: alias.resolve_controller_by_alias で 1 つに決まる
        // - alias がない場合: resolve_controllers_for_html で複数候補を取得
        let controller_candidates: Vec<(String, bool)> = if let Some(alias_name) = alias.as_deref()
        {
            if let Some(controller) =
                self.index
                    .resolve_controller_by_alias(html_uri, line, alias_name)
            {
                // alias がある = controller as 構文 → this.foo を優先
                vec![(controller, true)]
            } else {
                Vec::new()
            }
        } else {
            // alias がない = $scope.foo 形式
            self.index
                .resolve_controllers_for_html(html_uri, line)
                .into_iter()
                .map(|name| (name, false))
                .collect()
        };

        for (controller_name, prefer_this) in controller_candidates {
            // controller の JS 定義 (ControllerScope) を取得
            let scopes = self
                .index
                .controllers
                .get_controller_scopes_by_name(&controller_name);

            for scope in scopes {
                if let Some(action) = build_insert_property_action(
                    &controller_name,
                    &property,
                    prefer_this,
                    &scope,
                    diagnostic,
                    sources,
                ) {
                    actions.push(CodeActionOrCommand::CodeAction(action));
                }
            }
        }

        actions
    }
}

/// Diagnostic に [`UNDEFINED_SCOPE_PROPERTY_CODE`] が付与されているか判定する。
fn is_undefined_scope_property_diagnostic(diagnostic: &tower_lsp::lsp_types::Diagnostic) -> bool {
    matches!(
        &diagnostic.code,
        Some(NumberOrString::String(s)) if s == UNDEFINED_SCOPE_PROPERTY_CODE
    )
}

/// 2 つの Range が重なっているかを判定する。
/// `start1 <= end2 && start2 <= end1` の形式 (両端含む)。
fn ranges_overlap(a: &Range, b: &Range) -> bool {
    !position_lt(&b.end, &a.start) && !position_lt(&a.end, &b.start)
}

fn position_lt(a: &Position, b: &Position) -> bool {
    if a.line != b.line {
        a.line < b.line
    } else {
        a.character < b.character
    }
}

/// "alias.property" / "property" を分解する。
/// 例:
/// - `"vm.foo"` → `(Some("vm"), "foo")`
/// - `"vm.user.name"` → `(Some("vm"), "user.name")` (depth が深い場合は最深部まで保持)
/// - `"foo"` → `(None, "foo")`
fn split_alias_and_property(property_path: &str) -> (Option<String>, String) {
    if let Some(idx) = property_path.find('.') {
        let alias = property_path[..idx].to_string();
        let prop = property_path[idx + 1..].to_string();
        (Some(alias), prop)
    } else {
        (None, property_path.to_string())
    }
}

/// 「property tail」 を抽出する。
/// 挿入するのは scope 直下の最初の segment のみ (例: `"user.name"` → `"user"`)。
fn property_root_segment(property: &str) -> &str {
    match property.find('.') {
        Some(idx) => &property[..idx],
        None => property,
    }
}

/// controller 内に `this.X = null;` または `$scope.X = null;` を挿入する CodeAction を構築する。
fn build_insert_property_action(
    controller_name: &str,
    property: &str,
    prefer_this: bool,
    scope: &ControllerScope,
    diagnostic: &tower_lsp::lsp_types::Diagnostic,
    sources: &HashMap<Url, String>,
) -> Option<CodeAction> {
    // ネストした property の場合は root segment のみを挿入
    // 例: `vm.user.name` → `this.user = null;` (深い構造の作成は手動)
    let prop_to_insert = property_root_segment(property);
    if prop_to_insert.is_empty() {
        return None;
    }

    // 挿入位置: function body の **冒頭** (= body_start_line の **次の行**先頭)
    // 既存の最初の statement の前に新しい行を差し込む形になる。
    let insert_line = scope.start_line.saturating_add(1);

    // インデント推定: 利用可能なら body 先頭行の leading whitespace を流用、
    // ダメなら 4 spaces をデフォルトにする。
    let indent =
        detect_indent_from_source(sources.get(&scope.uri), insert_line).unwrap_or_else(|| {
            "    ".to_string()
        });

    let prefix = if prefer_this { "this" } else { "$scope" };
    let new_text = format!("{}{}.{} = null;\n", indent, prefix, prop_to_insert);

    let title = format!(
        "Add `{}.{} = null;` to controller `{}`",
        prefix, prop_to_insert, controller_name
    );

    let edit = TextEdit {
        range: Range {
            start: Position {
                line: insert_line,
                character: 0,
            },
            end: Position {
                line: insert_line,
                character: 0,
            },
        },
        new_text,
    };

    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    changes.insert(scope.uri.clone(), vec![edit]);

    Some(CodeAction {
        title,
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        command: None,
        is_preferred: Some(true),
        disabled: None,
        data: None,
    })
}

/// 指定行のソースから leading whitespace を抽出してインデント文字列として返す。
/// `source` が `None`、行が存在しない、または空行で leading whitespace を含まない
/// 場合は `None` を返す。
fn detect_indent_from_source(source: Option<&String>, line: u32) -> Option<String> {
    let src = source?;
    let target = src.lines().nth(line as usize)?;
    let indent: String = target
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    if indent.is_empty() {
        None
    } else {
        Some(indent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_alias_and_property_handles_dot() {
        assert_eq!(
            split_alias_and_property("vm.foo"),
            (Some("vm".to_string()), "foo".to_string())
        );
        assert_eq!(
            split_alias_and_property("vm.user.name"),
            (Some("vm".to_string()), "user.name".to_string())
        );
        assert_eq!(
            split_alias_and_property("foo"),
            (None, "foo".to_string())
        );
    }

    #[test]
    fn property_root_segment_returns_first_segment() {
        assert_eq!(property_root_segment("foo"), "foo");
        assert_eq!(property_root_segment("user.name"), "user");
        assert_eq!(property_root_segment("a.b.c"), "a");
    }

    #[test]
    fn ranges_overlap_works() {
        let r = |sl, sc, el, ec| Range {
            start: Position {
                line: sl,
                character: sc,
            },
            end: Position {
                line: el,
                character: ec,
            },
        };
        // 完全一致
        assert!(ranges_overlap(&r(1, 0, 1, 5), &r(1, 0, 1, 5)));
        // 重なる
        assert!(ranges_overlap(&r(1, 0, 1, 10), &r(1, 5, 1, 15)));
        // a が b の前
        assert!(!ranges_overlap(&r(1, 0, 1, 5), &r(1, 6, 1, 10)));
        // a が b の後
        assert!(!ranges_overlap(&r(1, 10, 1, 15), &r(1, 0, 1, 5)));
        // 行違い
        assert!(!ranges_overlap(&r(1, 0, 1, 5), &r(2, 0, 2, 5)));
    }

    #[test]
    fn detect_indent_from_source_extracts_leading_whitespace() {
        let src = "function f() {\n    var a = 1;\n\tvar b = 2;\nno_indent;\n".to_string();
        assert_eq!(
            detect_indent_from_source(Some(&src), 1),
            Some("    ".to_string())
        );
        assert_eq!(
            detect_indent_from_source(Some(&src), 2),
            Some("\t".to_string())
        );
        assert_eq!(detect_indent_from_source(Some(&src), 3), None);
        // 範囲外
        assert_eq!(detect_indent_from_source(Some(&src), 100), None);
        assert_eq!(detect_indent_from_source(None, 1), None);
    }
}
