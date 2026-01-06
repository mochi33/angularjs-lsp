use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::index::SymbolIndex;

pub struct ReferencesHandler {
    index: Arc<SymbolIndex>,
}

impl ReferencesHandler {
    pub fn new(index: Arc<SymbolIndex>) -> Self {
        Self { index }
    }

    pub fn find_references(&self, params: ReferenceParams) -> Option<Vec<Location>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let include_declaration = params.context.include_declaration;

        let symbol_name = self.index.find_symbol_at_position(
            &uri,
            position.line,
            position.character,
        )?;

        let mut locations = Vec::new();

        // Add definition locations if requested
        if include_declaration {
            for def in self.index.get_definitions(&symbol_name) {
                locations.push(Location {
                    uri: def.uri.clone(),
                    range: Range {
                        start: Position {
                            line: def.start_line,
                            character: def.start_col,
                        },
                        end: Position {
                            line: def.end_line,
                            character: def.end_col,
                        },
                    },
                });
            }
        }

        // Add reference locations
        for reference in self.index.get_references(&symbol_name) {
            locations.push(Location {
                uri: reference.uri.clone(),
                range: Range {
                    start: Position {
                        line: reference.start_line,
                        character: reference.start_col,
                    },
                    end: Position {
                        line: reference.end_line,
                        character: reference.end_col,
                    },
                },
            });
        }

        if locations.is_empty() {
            None
        } else {
            Some(locations)
        }
    }

    pub fn goto_definition(&self, params: GotoDefinitionParams) -> Option<GotoDefinitionResponse> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let symbol_name = self.index.find_symbol_at_position(
            &uri,
            position.line,
            position.character,
        )?;

        let definitions = self.index.get_definitions(&symbol_name);

        if definitions.is_empty() {
            return None;
        }

        let locations: Vec<Location> = definitions
            .into_iter()
            .map(|def| Location {
                uri: def.uri.clone(),
                range: Range {
                    start: Position {
                        line: def.start_line,
                        character: def.start_col,
                    },
                    end: Position {
                        line: def.end_line,
                        character: def.end_col,
                    },
                },
            })
            .collect();

        Some(GotoDefinitionResponse::Array(locations))
    }
}
