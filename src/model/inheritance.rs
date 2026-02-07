use serde::{Deserialize, Serialize};
use tower_lsp::lsp_types::Url;

use super::html::{InheritedFormBinding, InheritedLocalVariable};

/// ng-includeによる親子HTML関係
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NgIncludeBinding {
    pub parent_uri: Url,
    pub template_path: String,
    /// 親ファイルを起点として解決した絶対パス（ファイル名のみ）
    pub resolved_filename: String,
    /// ng-includeがある行
    pub line: u32,
    /// ng-includeがある位置での継承コントローラーリスト（外側から内側への順）
    pub inherited_controllers: Vec<String>,
    /// ng-includeがある位置での継承ローカル変数リスト
    pub inherited_local_variables: Vec<InheritedLocalVariable>,
    /// ng-includeがある位置での継承フォームバインディングリスト
    pub inherited_form_bindings: Vec<InheritedFormBinding>,
}

/// ng-viewによるルーティングテンプレートの継承関係
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NgViewBinding {
    /// ng-viewがあるHTMLファイルのURI
    pub parent_uri: Url,
    /// ng-viewがある行
    pub line: u32,
    /// ng-view位置での継承コントローラーリスト
    pub inherited_controllers: Vec<String>,
    /// ng-view位置での継承ローカル変数リスト
    pub inherited_local_variables: Vec<InheritedLocalVariable>,
    /// ng-view位置での継承フォームバインディングリスト
    pub inherited_form_bindings: Vec<InheritedFormBinding>,
}
