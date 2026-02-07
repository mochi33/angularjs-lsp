use serde::{Deserialize, Serialize};
use tower_lsp::lsp_types::Url;

/// コントローラーのスコープ情報（JSファイル側）
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ControllerScope {
    pub name: String,
    pub uri: Url,
    pub start_line: u32,
    pub end_line: u32,
    /// DIで注入されているサービス名のリスト
    pub injected_services: Vec<String>,
}

/// HTML内のng-controllerスコープ
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HtmlControllerScope {
    pub controller_name: String,
    /// "controller as alias"構文で指定されたalias名（例: "formCustomItem"）
    pub alias: Option<String>,
    pub uri: Url,
    pub start_line: u32,
    pub end_line: u32,
}

/// DIスコープ（アナライザーコンテキスト用）
#[derive(Clone, Debug, Default)]
pub struct DiScope {
    pub controller_name: Option<String>,
    pub injected_services: Vec<String>,
    pub has_scope: bool,
    pub has_root_scope: bool,
}
