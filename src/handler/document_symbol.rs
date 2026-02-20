use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::index::Index;

pub struct DocumentSymbolHandler {
    index: Arc<Index>,
}

impl DocumentSymbolHandler {
    pub fn new(index: Arc<Index>) -> Self {
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
                let range_start_line = s
                    .definition_span
                    .start_line
                    .min(s.name_span.start_line);
                let range_start_col = if range_start_line == s.definition_span.start_line
                    && range_start_line == s.name_span.start_line
                {
                    s.definition_span.start_col.min(s.name_span.start_col)
                } else if range_start_line == s.definition_span.start_line {
                    s.definition_span.start_col
                } else {
                    s.name_span.start_col
                };
                let range_end_line =
                    s.definition_span.end_line.max(s.name_span.end_line);
                let range_end_col = if range_end_line == s.definition_span.end_line
                    && range_end_line == s.name_span.end_line
                {
                    s.definition_span.end_col.max(s.name_span.end_col)
                } else if range_end_line == s.definition_span.end_line {
                    s.definition_span.end_col
                } else {
                    s.name_span.end_col
                };

                #[allow(deprecated)]
                DocumentSymbol {
                    name: s.name.clone(),
                    detail: Some(s.kind.as_str().to_string()),
                    kind: s.kind.to_lsp_symbol_kind(),
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
                    selection_range: s.name_span.to_lsp_range(),
                    children: None,
                }
            })
            .collect();

        Some(DocumentSymbolResponse::Nested(document_symbols))
    }

}
