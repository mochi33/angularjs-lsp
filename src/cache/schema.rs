use serde::{Deserialize, Serialize};

use crate::model::{
    HtmlControllerScope, HtmlDirectiveReference, HtmlFormBinding, HtmlLocalVariable,
    HtmlLocalVariableReference, HtmlScopeReference, NgIncludeBinding, Symbol, SymbolReference,
    ControllerScope, TemplateBinding,
};

/// Cached per-file symbol data
#[derive(Serialize, Deserialize)]
pub struct CachedSymbolData {
    pub uri: String,
    pub definitions: Vec<Symbol>,
    pub references: Vec<SymbolReference>,
    pub controller_scopes: Vec<ControllerScope>,
    #[serde(default)]
    pub html_controller_scopes: Vec<HtmlControllerScope>,
    #[serde(default)]
    pub html_scope_references: Vec<HtmlScopeReference>,
    #[serde(default)]
    pub html_local_variables: Vec<HtmlLocalVariable>,
    #[serde(default)]
    pub html_local_variable_references: Vec<HtmlLocalVariableReference>,
    #[serde(default)]
    pub html_form_bindings: Vec<HtmlFormBinding>,
    #[serde(default)]
    pub html_directive_references: Vec<HtmlDirectiveReference>,
}

/// Cached global data (not file-specific)
#[derive(Serialize, Deserialize)]
pub struct CachedGlobalData {
    pub template_bindings: Vec<TemplateBinding>,
    pub ng_include_bindings: Vec<(String, NgIncludeBinding)>,
    /// JS から検出された `$interpolateProvider.startSymbol/endSymbol` の値を
    /// URI 単位で永続化する。`(uri_str, start_symbol, end_symbol)` の Vec。
    /// 各 URI で start/end どちらか片方だけ宣言されているケースもあり得るので
    /// それぞれ Option で保持する。
    ///
    /// このフィールドが無いとキャッシュからの起動時に `InterpolateStore` が
    /// 空になり、custom interpolate 記号を使うプロジェクトで HTML 解析が
    /// デフォルト `{{ }}` で動いてしまう。
    pub interpolate_symbols: Vec<(String, Option<String>, Option<String>)>,
}
