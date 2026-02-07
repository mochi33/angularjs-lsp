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
}
