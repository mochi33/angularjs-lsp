use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::index::Index;
use crate::model::SymbolKind as AngularSymbolKind;

pub struct WorkspaceSymbolHandler {
    index: Arc<Index>,
}

impl WorkspaceSymbolHandler {
    pub fn new(index: Arc<Index>) -> Self {
        Self { index }
    }

    pub fn handle(&self, query: &str) -> Vec<SymbolInformation> {
        let all_definitions = self.index.definitions.get_all_definitions();
        let query_lower = query.to_lowercase();

        all_definitions
            .into_iter()
            .filter(|sym| self.is_top_level_symbol(sym.kind))
            .filter(|sym| query.is_empty() || sym.name.to_lowercase().contains(&query_lower))
            .map(|sym| {
                #[allow(deprecated)]
                SymbolInformation {
                    name: sym.name.clone(),
                    kind: sym.kind.to_lsp_symbol_kind(),
                    tags: None,
                    deprecated: None,
                    location: Location {
                        uri: sym.uri.clone(),
                        range: sym.definition_span.to_lsp_range(),
                    },
                    container_name: Some(sym.kind.as_str().to_string()),
                }
            })
            .collect()
    }

    fn is_top_level_symbol(&self, kind: AngularSymbolKind) -> bool {
        matches!(
            kind,
            AngularSymbolKind::Module
                | AngularSymbolKind::Controller
                | AngularSymbolKind::Service
                | AngularSymbolKind::Factory
                | AngularSymbolKind::Directive
                | AngularSymbolKind::Component
                | AngularSymbolKind::Provider
                | AngularSymbolKind::Filter
                | AngularSymbolKind::Constant
                | AngularSymbolKind::Value
        )
    }
}
