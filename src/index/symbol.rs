use serde::{Deserialize, Serialize};
use tower_lsp::lsp_types::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SymbolKind {
    Module,
    Controller,
    Service,
    Factory,
    Directive,
    Provider,
    Filter,
    Constant,
    Value,
    Method,
    /// $scope プロパティ
    ScopeProperty,
    /// $scope メソッド（関数が格納されている）
    ScopeMethod,
    /// $rootScope プロパティ
    RootScopeProperty,
    /// $rootScope メソッド（関数が格納されている）
    RootScopeMethod,
    /// <form name="x">で$scopeに自動バインドされるフォーム
    FormBinding,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Module => "module",
            SymbolKind::Controller => "controller",
            SymbolKind::Service => "service",
            SymbolKind::Factory => "factory",
            SymbolKind::Directive => "directive",
            SymbolKind::Provider => "provider",
            SymbolKind::Filter => "filter",
            SymbolKind::Constant => "constant",
            SymbolKind::Value => "value",
            SymbolKind::Method => "method",
            SymbolKind::ScopeProperty => "scope property",
            SymbolKind::ScopeMethod => "scope method",
            SymbolKind::RootScopeProperty => "root scope property",
            SymbolKind::RootScopeMethod => "root scope method",
            SymbolKind::FormBinding => "form binding",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub uri: Url,
    /// 定義位置（ジャンプ先）- 関数全体の開始位置など
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    /// シンボル名の位置（検索用）- シンボル名が記述されている正確な位置
    pub name_start_line: u32,
    pub name_start_col: u32,
    pub name_end_line: u32,
    pub name_end_col: u32,
    pub docs: Option<String>,
    /// 関数パラメータ（ScopeMethodやMethodなどの場合）
    pub parameters: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolReference {
    pub name: String,
    pub uri: Url,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}
