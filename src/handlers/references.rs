use std::sync::Arc;

use tower_lsp::lsp_types::*;
use tracing::debug;

use crate::index::SymbolIndex;

pub struct ReferencesHandler {
    index: Arc<SymbolIndex>,
}

impl ReferencesHandler {
    pub fn new(index: Arc<SymbolIndex>) -> Self {
        Self { index }
    }

    /// ファイルがHTMLかどうか判定
    fn is_html_file(uri: &Url) -> bool {
        let path = uri.path().to_lowercase();
        path.ends_with(".html") || path.ends_with(".htm")
    }

    pub fn find_references(&self, params: ReferenceParams) -> Option<Vec<Location>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let include_declaration = params.context.include_declaration;

        // HTMLファイルの場合は専用の処理
        if Self::is_html_file(&uri) {
            return self.find_references_from_html(&uri, position, include_declaration);
        }

        let symbol_name = self.index.find_symbol_at_position(
            &uri,
            position.line,
            position.character,
        )?;

        self.collect_references(&symbol_name, include_declaration)
    }

    /// HTMLファイルからの参照検索
    fn find_references_from_html(
        &self,
        uri: &Url,
        position: Position,
        include_declaration: bool,
    ) -> Option<Vec<Location>> {
        // 1. 位置からHTMLスコープ参照を取得
        let html_ref = self.index.find_html_scope_reference_at(
            uri,
            position.line,
            position.character,
        )?;

        debug!(
            "find_references_from_html: found reference '{}' at {}:{}",
            html_ref.property_path, position.line, position.character
        );

        // 2. コントローラー名を解決（複数の可能性あり）
        let controllers = self.index.resolve_controllers_for_html(uri, position.line);

        // 3. 各コントローラーを順番に試して、定義が見つかったものを返す
        for controller_name in controllers {
            debug!(
                "find_references_from_html: trying controller '{}'",
                controller_name
            );

            let symbol_name = format!(
                "{}.$scope.{}",
                controller_name,
                html_ref.property_path
            );

            debug!(
                "find_references_from_html: looking up symbol '{}'",
                symbol_name
            );

            if let Some(locations) = self.collect_references(&symbol_name, include_declaration) {
                return Some(locations);
            }
        }

        None
    }

    /// シンボル名から定義と参照を収集
    fn collect_references(&self, symbol_name: &str, include_declaration: bool) -> Option<Vec<Location>> {
        let mut locations = Vec::new();

        // Add definition locations if requested
        if include_declaration {
            for def in self.index.get_definitions(symbol_name) {
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

        // Add reference locations (JS + HTML)
        for reference in self.index.get_all_references(symbol_name) {
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

        // HTMLファイルの場合は専用の処理
        if Self::is_html_file(&uri) {
            return self.goto_definition_from_html(&uri, position);
        }

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

    /// HTMLファイルからの定義ジャンプ
    fn goto_definition_from_html(&self, uri: &Url, position: Position) -> Option<GotoDefinitionResponse> {
        // 1. 位置からHTMLスコープ参照を取得
        let html_ref = self.index.find_html_scope_reference_at(
            uri,
            position.line,
            position.character,
        )?;

        debug!(
            "goto_definition_from_html: found reference '{}' at {}:{}",
            html_ref.property_path, position.line, position.character
        );

        // 2. コントローラー名を解決（複数の可能性あり）
        let controllers = self.index.resolve_controllers_for_html(uri, position.line);

        // 3. 各コントローラーを順番に試して、定義が見つかったものを返す
        for controller_name in controllers {
            debug!(
                "goto_definition_from_html: trying controller '{}'",
                controller_name
            );

            let symbol_name = format!(
                "{}.$scope.{}",
                controller_name,
                html_ref.property_path
            );

            debug!(
                "goto_definition_from_html: looking up symbol '{}'",
                symbol_name
            );

            let definitions = self.index.get_definitions(&symbol_name);

            if !definitions.is_empty() {
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

                debug!(
                    "goto_definition_from_html: found {} locations",
                    locations.len()
                );

                return Some(GotoDefinitionResponse::Array(locations));
            }
        }

        debug!("goto_definition_from_html: no definitions found in any controller");
        None
    }
}
