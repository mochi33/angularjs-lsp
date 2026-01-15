use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::index::SymbolKind as AngularSymbolKind;
use crate::index::SymbolIndex;

pub struct DocumentSymbolHandler {
    index: Arc<SymbolIndex>,
}

impl DocumentSymbolHandler {
    pub fn new(index: Arc<SymbolIndex>) -> Self {
        Self { index }
    }

    pub fn document_symbols(&self, uri: &Url) -> Option<DocumentSymbolResponse> {
        let symbols = self.index.get_document_symbols(uri);

        if symbols.is_empty() {
            return None;
        }

        let document_symbols: Vec<DocumentSymbol> = symbols
            .into_iter()
            .map(|s| {
                // Ensure selection_range is contained within range
                // LSP requires: range.start <= selection_range.start && selection_range.end <= range.end
                let range_start_line = s.start_line.min(s.name_start_line);
                let range_start_col = if range_start_line == s.start_line && range_start_line == s.name_start_line {
                    s.start_col.min(s.name_start_col)
                } else if range_start_line == s.start_line {
                    s.start_col
                } else {
                    s.name_start_col
                };
                let range_end_line = s.end_line.max(s.name_end_line);
                let range_end_col = if range_end_line == s.end_line && range_end_line == s.name_end_line {
                    s.end_col.max(s.name_end_col)
                } else if range_end_line == s.end_line {
                    s.end_col
                } else {
                    s.name_end_col
                };

                #[allow(deprecated)]
                DocumentSymbol {
                    name: s.name.clone(),
                    detail: Some(s.kind.as_str().to_string()),
                    kind: self.convert_symbol_kind(s.kind),
                    tags: None,
                    deprecated: None,
                    range: Range {
                        start: Position {
                            line: range_start_line,
                            character: range_start_col,
                        },
                        end: Position {
                            line: range_end_line,
                            character: range_end_col,
                        },
                    },
                    selection_range: Range {
                        start: Position {
                            line: s.name_start_line,
                            character: s.name_start_col,
                        },
                        end: Position {
                            line: s.name_end_line,
                            character: s.name_end_col,
                        },
                    },
                    children: None,
                }
            })
            .collect();

        Some(DocumentSymbolResponse::Nested(document_symbols))
    }

    fn convert_symbol_kind(&self, kind: AngularSymbolKind) -> SymbolKind {
        match kind {
            AngularSymbolKind::Module => SymbolKind::MODULE,
            AngularSymbolKind::Controller => SymbolKind::CLASS,
            AngularSymbolKind::Service => SymbolKind::CLASS,
            AngularSymbolKind::Factory => SymbolKind::CLASS,
            AngularSymbolKind::Directive => SymbolKind::CLASS,
            AngularSymbolKind::Provider => SymbolKind::CLASS,
            AngularSymbolKind::Filter => SymbolKind::FUNCTION,
            AngularSymbolKind::Constant => SymbolKind::CONSTANT,
            AngularSymbolKind::Value => SymbolKind::VARIABLE,
            AngularSymbolKind::Method => SymbolKind::METHOD,
            AngularSymbolKind::ScopeProperty => SymbolKind::PROPERTY,
            AngularSymbolKind::ScopeMethod => SymbolKind::METHOD,
            AngularSymbolKind::RootScopeProperty => SymbolKind::PROPERTY,
            AngularSymbolKind::RootScopeMethod => SymbolKind::METHOD,
            AngularSymbolKind::FormBinding => SymbolKind::VARIABLE,
            AngularSymbolKind::ExportedComponent => SymbolKind::CLASS,
        }
    }
}
