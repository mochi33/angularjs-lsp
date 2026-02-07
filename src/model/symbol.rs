use serde::{Deserialize, Serialize};
use tower_lsp::lsp_types::Url;

use super::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SymbolKind {
    Module,
    Controller,
    Service,
    Factory,
    Directive,
    /// AngularJS 1.5+ コンポーネント（.component() で登録）
    Component,
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
    /// ES6 export default で公開されたコンポーネント
    ExportedComponent,
    /// コンポーネントのbindingsプロパティ（'<', '=', '@', '&'）
    ComponentBinding,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Module => "module",
            SymbolKind::Controller => "controller",
            SymbolKind::Service => "service",
            SymbolKind::Factory => "factory",
            SymbolKind::Directive => "directive",
            SymbolKind::Component => "component",
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
            SymbolKind::ExportedComponent => "exported component",
            SymbolKind::ComponentBinding => "component binding",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub uri: Url,
    /// 定義位置（ジャンプ先）- 関数全体の開始位置など
    pub definition_span: Span,
    /// シンボル名の位置（検索用）- シンボル名が記述されている正確な位置
    pub name_span: Span,
    pub docs: Option<String>,
    /// 関数パラメータ（ScopeMethodやMethodなどの場合）
    pub parameters: Option<Vec<String>>,
}

impl Symbol {
    /// 旧フォーマットとの互換アクセサ
    pub fn start_line(&self) -> u32 {
        self.definition_span.start_line
    }
    pub fn start_col(&self) -> u32 {
        self.definition_span.start_col
    }
    pub fn end_line(&self) -> u32 {
        self.definition_span.end_line
    }
    pub fn end_col(&self) -> u32 {
        self.definition_span.end_col
    }
    pub fn name_start_line(&self) -> u32 {
        self.name_span.start_line
    }
    pub fn name_start_col(&self) -> u32 {
        self.name_span.start_col
    }
    pub fn name_end_line(&self) -> u32 {
        self.name_span.end_line
    }
    pub fn name_end_col(&self) -> u32 {
        self.name_span.end_col
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolReference {
    pub name: String,
    pub uri: Url,
    pub span: Span,
}

impl SymbolReference {
    /// 旧フォーマットとの互換アクセサ
    pub fn start_line(&self) -> u32 {
        self.span.start_line
    }
    pub fn start_col(&self) -> u32 {
        self.span.start_col
    }
    pub fn end_line(&self) -> u32 {
        self.span.end_line
    }
    pub fn end_col(&self) -> u32 {
        self.span.end_col
    }
}
