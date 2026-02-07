use serde::{Deserialize, Serialize};
use tower_lsp::lsp_types::Url;

/// ES6 export default で公開されたコンポーネントの情報
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportInfo {
    pub uri: Url,
    /// コンポーネント名（関数参照名またはファイル名から導出）
    pub component_name: String,
    /// DI配列からの依存関係
    pub dependencies: Vec<String>,
    /// export文の開始位置
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    /// $scopeを依存に持つか
    pub has_scope: bool,
    /// $rootScopeを依存に持つか
    pub has_root_scope: bool,
}

/// ES6 export default { name: 'xxx', config: {...} } パターンの情報
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportedComponentObject {
    pub uri: Url,
    /// nameプロパティの値（例: 'userDetails'）
    pub name: String,
    /// export文の開始位置
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}
