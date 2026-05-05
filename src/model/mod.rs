pub mod builder;
pub mod export;
pub mod html;
pub mod inheritance;
pub mod scope;
pub mod span;
pub mod symbol;
pub mod template;

pub use builder::SymbolBuilder;
pub use export::{ExportInfo, ExportedComponentObject};
pub use html::{
    DirectiveUsageType, HtmlComponentAttribute, HtmlComponentUsage, HtmlDirectiveReference,
    HtmlFormBinding, HtmlLocalVariable, HtmlLocalVariableReference, HtmlLocalVariableSource,
    HtmlNgModelTarget, HtmlScopeReference, HtmlUiSrefReference, InheritedFormBinding,
    InheritedLocalVariable,
};
pub use inheritance::{NgIncludeBinding, NgViewBinding};
pub use scope::{ControllerScope, HtmlControllerScope};
pub use span::Span;
pub use symbol::{Symbol, SymbolKind, SymbolReference, COMPONENT_BINDING_DOCS_PREFIX};
pub use template::{BindingSource, ComponentTemplateUrl, TemplateBinding};
