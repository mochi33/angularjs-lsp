//! `ng-model` の暗黙的な `$scope` 定義収集
//!
//! `<input ng-model="user.name">` は `$scope.user.name` への双方向書き込みを生むので、
//! controller 側で `$scope.user.name = ...` を書いていなくても診断で「未定義」と判定
//! しないように、テンプレート側で暗黙的な定義として記録する。
//!
//! `$scope` 参照収集 (`scope_reference.rs`) とは責務が別なので独立モジュールに分離
//! している (issue #48)。

use tower_lsp::lsp_types::Url;

use crate::model::HtmlNgModelTarget;

use super::HtmlAngularJsAnalyzer;

impl HtmlAngularJsAnalyzer {
    /// `ng-model="X"` の値 `X` を `$scope` への暗黙的な書き込み定義として登録する。
    pub(super) fn register_ng_model_target(
        &self,
        uri: &Url,
        value: &str,
        value_start_line: u32,
        value_start_col: u32,
    ) {
        let len_utf16 = value.chars().map(|c| c.len_utf16()).sum::<usize>() as u32;
        let target = HtmlNgModelTarget {
            property_path: value.to_string(),
            uri: uri.clone(),
            start_line: value_start_line,
            start_col: value_start_col,
            end_line: value_start_line,
            end_col: value_start_col + len_utf16,
        };
        self.index.html.add_ng_model_target(target);
    }
}
