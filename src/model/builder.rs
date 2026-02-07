use tower_lsp::lsp_types::Url;

use super::span::Span;
use super::symbol::{Symbol, SymbolKind, SymbolReference};

/// Symbol構築のビルダーパターン
pub struct SymbolBuilder {
    name: String,
    kind: SymbolKind,
    uri: Url,
    definition_span: Span,
    name_span: Span,
    docs: Option<String>,
    parameters: Option<Vec<String>>,
}

impl SymbolBuilder {
    pub fn new(name: impl Into<String>, kind: SymbolKind, uri: Url) -> Self {
        Self {
            name: name.into(),
            kind,
            uri,
            definition_span: Span::default(),
            name_span: Span::default(),
            docs: None,
            parameters: None,
        }
    }

    pub fn definition_span(mut self, span: Span) -> Self {
        self.definition_span = span;
        self
    }

    pub fn name_span(mut self, span: Span) -> Self {
        self.name_span = span;
        self
    }

    pub fn docs(mut self, docs: impl Into<String>) -> Self {
        self.docs = Some(docs.into());
        self
    }

    pub fn parameters(mut self, params: Vec<String>) -> Self {
        self.parameters = Some(params);
        self
    }

    pub fn build(self) -> Symbol {
        Symbol {
            name: self.name,
            kind: self.kind,
            uri: self.uri,
            definition_span: self.definition_span,
            name_span: self.name_span,
            docs: self.docs,
            parameters: self.parameters,
        }
    }
}

/// SymbolReference構築のビルダーパターン
pub struct ReferenceBuilder {
    name: String,
    uri: Url,
    span: Span,
}

impl ReferenceBuilder {
    pub fn new(name: impl Into<String>, uri: Url) -> Self {
        Self {
            name: name.into(),
            uri,
            span: Span::default(),
        }
    }

    pub fn span(mut self, span: Span) -> Self {
        self.span = span;
        self
    }

    pub fn build(self) -> SymbolReference {
        SymbolReference {
            name: self.name,
            uri: self.uri,
            span: self.span,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_builder() {
        let uri = Url::parse("file:///test.js").unwrap();
        let symbol = SymbolBuilder::new("MyCtrl.$scope.name", SymbolKind::ScopeProperty, uri.clone())
            .definition_span(Span::new(10, 0, 10, 30))
            .name_span(Span::new(10, 8, 10, 12))
            .docs("A scope property")
            .build();

        assert_eq!(symbol.name, "MyCtrl.$scope.name");
        assert_eq!(symbol.kind, SymbolKind::ScopeProperty);
        assert_eq!(symbol.uri, uri);
        assert_eq!(symbol.definition_span, Span::new(10, 0, 10, 30));
        assert_eq!(symbol.name_span, Span::new(10, 8, 10, 12));
        assert_eq!(symbol.docs.as_deref(), Some("A scope property"));
    }

    #[test]
    fn test_reference_builder() {
        let uri = Url::parse("file:///test.html").unwrap();
        let reference = ReferenceBuilder::new("MyCtrl", uri.clone())
            .span(Span::new(5, 10, 5, 16))
            .build();

        assert_eq!(reference.name, "MyCtrl");
        assert_eq!(reference.uri, uri);
        assert_eq!(reference.span, Span::new(5, 10, 5, 16));
    }
}
