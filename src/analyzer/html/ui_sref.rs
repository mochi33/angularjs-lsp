//! `ui-sref` (ui-router) 属性値からの state 名参照収集
//!
//! ui-router は `<a ui-sref="home">` のようにテンプレートから state へジャンプするので、
//! state 名を参照として登録しておくことで goto-definition / find-references が機能する。
//!
//! `$scope` 参照とは独立した責務なので独立モジュールに分離している (issue #48)。

use tower_lsp::lsp_types::Url;

use crate::model::{HtmlUiSrefReference, Span, SymbolReference};

use super::HtmlAngularJsAnalyzer;

impl HtmlAngularJsAnalyzer {
    /// `ui-sref="state-name[(...args)]"` の state 名部分を `HtmlUiSrefReference` と
    /// `SymbolReference` の両方として登録する。
    ///
    /// 値の形式:
    /// - `home` → state 名 = "home"
    /// - `home.detail` → state 名 = "home.detail" (dot-notation 子 state)
    /// - `home({id: 1})` → state 名 = "home" (引数部分は無視)
    ///
    /// 以下はスキップ:
    /// - 空文字列
    /// - 相対参照 (`.`, `^`, `^.foo` など) — state 名解決に親 state コンテキストが必要
    pub(super) fn register_ui_sref_reference(
        &self,
        uri: &Url,
        value: &str,
        value_start_line: u32,
        value_start_col: u32,
    ) {
        // 引数部分 `(...)` の前までを state 名として切り出す
        let name_part = value.split('(').next().unwrap_or(value);
        let trimmed = name_part.trim();
        if trimmed.is_empty() {
            return;
        }

        // ui-router の相対参照は今は解決対象外
        if trimmed == "." || trimmed.starts_with('^') {
            return;
        }

        // 先頭の空白文字数だけ start_col をずらす
        let leading_whitespace_bytes = name_part.len() - name_part.trim_start().len();
        let utf16_leading = self.byte_offset_to_utf16_offset(name_part, leading_whitespace_bytes) as u32;
        let start_col = value_start_col + utf16_leading;
        let len_utf16 = trimmed.chars().map(|c| c.len_utf16()).sum::<usize>() as u32;
        let end_col = start_col + len_utf16;

        let reference = HtmlUiSrefReference {
            state_name: trimmed.to_string(),
            uri: uri.clone(),
            start_line: value_start_line,
            start_col,
            end_line: value_start_line,
            end_col,
        };
        self.index.html.add_ui_sref_reference(reference);

        // 既存の find-references インフラ用に SymbolReference も登録する
        // (これにより state 定義からの find-references で HTML 側の使用箇所も列挙される)
        self.index.definitions.add_reference(SymbolReference {
            name: trimmed.to_string(),
            uri: uri.clone(),
            span: Span::new(value_start_line, start_col, value_start_line, end_col),
        });
    }
}
