use std::collections::HashMap;
use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::index::SymbolIndex;

pub struct RenameHandler {
    index: Arc<SymbolIndex>,
}

impl RenameHandler {
    pub fn new(index: Arc<SymbolIndex>) -> Self {
        Self { index }
    }

    /// ファイルがHTMLかどうか判定
    fn is_html_file(uri: &Url) -> bool {
        let path = uri.path().to_lowercase();
        path.ends_with(".html") || path.ends_with(".htm")
    }

    pub fn rename(&self, params: RenameParams) -> Option<WorkspaceEdit> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_name = params.new_name;

        // HTMLファイルの場合は専用の処理
        let symbol_name = if Self::is_html_file(&uri) {
            self.resolve_symbol_name_from_html(&uri, position)?
        } else {
            self.index.find_symbol_at_position(
                &uri,
                position.line,
                position.character,
            )?
        };

        self.collect_edits(&symbol_name, &new_name)
    }

    /// HTMLファイルからシンボル名を解決
    fn resolve_symbol_name_from_html(&self, uri: &Url, position: Position) -> Option<String> {
        // 1. 位置からHTMLスコープ参照を取得
        let html_ref = self.index.find_html_scope_reference_at(
            uri,
            position.line,
            position.character,
        )?;

        // 2. コントローラー名を解決
        let controller_name = self.index.resolve_controller_for_html(uri, position.line)?;

        // 3. シンボル名を構築 "ControllerName.$scope.property"
        Some(format!(
            "{}.$scope.{}",
            controller_name,
            html_ref.property_path
        ))
    }

    /// シンボル名から編集を収集
    fn collect_edits(&self, symbol_name: &str, new_name: &str) -> Option<WorkspaceEdit> {
        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();

        // Collect definition locations (use name_* positions for accurate renaming)
        for def in self.index.get_definitions(symbol_name) {
            let edit = TextEdit {
                range: Range {
                    start: Position {
                        line: def.name_start_line,
                        character: def.name_start_col,
                    },
                    end: Position {
                        line: def.name_end_line,
                        character: def.name_end_col,
                    },
                },
                new_text: new_name.to_string(),
            };
            changes.entry(def.uri.clone()).or_default().push(edit);
        }

        // Collect reference locations
        for reference in self.index.get_references(symbol_name) {
            let edit = TextEdit {
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
                new_text: new_name.to_string(),
            };
            changes.entry(reference.uri.clone()).or_default().push(edit);
        }

        if changes.is_empty() {
            None
        } else {
            Some(WorkspaceEdit {
                changes: Some(changes),
                ..Default::default()
            })
        }
    }

    pub fn prepare_rename(&self, params: TextDocumentPositionParams) -> Option<PrepareRenameResponse> {
        let uri = params.text_document.uri;
        let position = params.position;

        // HTMLファイルの場合は専用の処理
        if Self::is_html_file(&uri) {
            return self.prepare_rename_from_html(&uri, position);
        }

        let symbol_name = self.index.find_symbol_at_position(
            &uri,
            position.line,
            position.character,
        )?;

        self.find_symbol_range_at_position(&symbol_name, &uri, position)
    }

    /// HTMLファイルからのprepare_rename
    fn prepare_rename_from_html(&self, uri: &Url, position: Position) -> Option<PrepareRenameResponse> {
        // HTMLスコープ参照を取得して、その範囲を返す
        let html_ref = self.index.find_html_scope_reference_at(
            uri,
            position.line,
            position.character,
        )?;

        Some(PrepareRenameResponse::Range(Range {
            start: Position {
                line: html_ref.start_line,
                character: html_ref.start_col,
            },
            end: Position {
                line: html_ref.end_line,
                character: html_ref.end_col,
            },
        }))
    }

    /// シンボルの範囲を見つける
    fn find_symbol_range_at_position(
        &self,
        symbol_name: &str,
        uri: &Url,
        position: Position,
    ) -> Option<PrepareRenameResponse> {
        // First check definitions
        for def in self.index.get_definitions(symbol_name) {
            if def.uri == *uri
                && position.line >= def.name_start_line
                && position.line <= def.name_end_line
            {
                let in_range = if def.name_start_line == def.name_end_line {
                    position.character >= def.name_start_col
                        && position.character <= def.name_end_col
                } else {
                    true
                };
                if in_range {
                    return Some(PrepareRenameResponse::Range(Range {
                        start: Position {
                            line: def.name_start_line,
                            character: def.name_start_col,
                        },
                        end: Position {
                            line: def.name_end_line,
                            character: def.name_end_col,
                        },
                    }));
                }
            }
        }

        // Then check references
        for reference in self.index.get_references(symbol_name) {
            if reference.uri == *uri
                && position.line >= reference.start_line
                && position.line <= reference.end_line
            {
                let in_range = if reference.start_line == reference.end_line {
                    position.character >= reference.start_col
                        && position.character <= reference.end_col
                } else {
                    true
                };
                if in_range {
                    return Some(PrepareRenameResponse::Range(Range {
                        start: Position {
                            line: reference.start_line,
                            character: reference.start_col,
                        },
                        end: Position {
                            line: reference.end_line,
                            character: reference.end_col,
                        },
                    }));
                }
            }
        }

        None
    }
}
