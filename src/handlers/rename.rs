use std::collections::HashMap;
use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::index::{HtmlFormBinding, HtmlLocalVariable, SymbolIndex};

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
        if Self::is_html_file(&uri) {
            // まずローカル変数をチェック（定義位置にカーソルがある場合）
            if let Some(local_var_def) = self.index.find_html_local_variable_definition_at(
                &uri,
                position.line,
                position.character,
            ) {
                return self.collect_local_variable_edits(&local_var_def, &new_name);
            }

            // ローカル変数参照をチェック
            if let Some(local_var_ref) = self.index.find_html_local_variable_at(
                &uri,
                position.line,
                position.character,
            ) {
                if let Some(var_def) = self.index.find_local_variable_definition(
                    &uri,
                    &local_var_ref.variable_name,
                    position.line,
                ) {
                    return self.collect_local_variable_edits(&var_def, &new_name);
                }
            }

            // フォームバインディングをチェック（定義位置にカーソルがある場合）
            if let Some(form_binding) = self.index.find_html_form_binding_at(
                &uri,
                position.line,
                position.character,
            ) {
                return self.collect_form_binding_edits(&uri, &form_binding, &new_name);
            }

            // 3. スコープ参照をチェック（継承されたローカル変数やフォームバインディングの可能性もある）
            if let Some(html_ref) = self.index.find_html_scope_reference_at(
                &uri,
                position.line,
                position.character,
            ) {
                let base_name = html_ref.property_path.split('.').next().unwrap_or(&html_ref.property_path);

                // フォームバインディング参照かどうかをチェック
                if let Some(form_binding) = self.index.find_form_binding_definition(
                    &uri,
                    base_name,
                    position.line,
                ) {
                    return self.collect_form_binding_edits(&uri, &form_binding, &new_name);
                }

                // 継承されたローカル変数かどうかをチェック
                if let Some(var_def) = self.index.find_local_variable_definition(
                    &uri,
                    base_name,
                    position.line,
                ) {
                    return self.collect_local_variable_edits(&var_def, &new_name);
                }
            }

            // 通常のスコープ参照
            let symbol_name = self.resolve_symbol_name_from_html(&uri, position)?;
            return self.collect_edits(&symbol_name, &new_name);
        }

        let symbol_name = self.index.find_symbol_at_position(
            &uri,
            position.line,
            position.character,
        )?;

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

        // 2. alias.property形式かチェック（controller as alias構文）
        let (resolved_controller, property_path) = if html_ref.property_path.contains('.') {
            let parts: Vec<&str> = html_ref.property_path.splitn(2, '.').collect();
            if parts.len() == 2 {
                let alias = parts[0];
                let prop = parts[1];
                // aliasからコントローラーを解決
                if let Some(controller) = self.index.resolve_controller_by_alias(uri, position.line, alias) {
                    (Some(controller), prop.to_string())
                } else {
                    (None, html_ref.property_path.clone())
                }
            } else {
                (None, html_ref.property_path.clone())
            }
        } else {
            (None, html_ref.property_path.clone())
        };

        // 3. コントローラー名を解決
        let controller_name = if let Some(ref controller) = resolved_controller {
            controller.clone()
        } else {
            self.index.resolve_controller_for_html(uri, position.line)?
        };

        // 4. シンボル名を構築 "ControllerName.$scope.property"
        let scope_symbol = format!(
            "{}.$scope.{}",
            controller_name,
            property_path
        );

        // まず$scope形式で定義が見つかるか確認
        if !self.index.get_definitions(&scope_symbol).is_empty() {
            return Some(scope_symbol);
        }

        // 5. controller as構文の場合、ControllerName.method形式も試す
        if resolved_controller.is_some() {
            let method_symbol = format!(
                "{}.{}",
                controller_name,
                property_path
            );
            if !self.index.get_definitions(&method_symbol).is_empty() {
                return Some(method_symbol);
            }
        }

        // デフォルトで$scope形式を返す
        Some(scope_symbol)
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
        for reference in self.index.get_all_references(symbol_name) {
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

    /// ローカル変数の編集を収集
    fn collect_local_variable_edits(
        &self,
        var_def: &HtmlLocalVariable,
        new_name: &str,
    ) -> Option<WorkspaceEdit> {
        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();

        // 定義位置の編集を追加
        let def_edit = TextEdit {
            range: Range {
                start: Position {
                    line: var_def.name_start_line,
                    character: var_def.name_start_col,
                },
                end: Position {
                    line: var_def.name_end_line,
                    character: var_def.name_end_col,
                },
            },
            new_text: new_name.to_string(),
        };
        changes
            .entry(var_def.uri.clone())
            .or_default()
            .push(def_edit);

        // スコープ内の全参照の編集を追加
        let refs = self.index.get_local_variable_references(
            &var_def.uri,
            &var_def.name,
            var_def.scope_start_line,
            var_def.scope_end_line,
        );

        for reference in refs {
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
            changes
                .entry(reference.uri.clone())
                .or_default()
                .push(edit);
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

    /// フォームバインディングの編集を収集
    fn collect_form_binding_edits(
        &self,
        uri: &Url,
        form_binding: &HtmlFormBinding,
        new_name: &str,
    ) -> Option<WorkspaceEdit> {
        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();

        // 定義位置（<form name="x">のname属性値）の編集を追加
        let def_edit = TextEdit {
            range: Range {
                start: Position {
                    line: form_binding.name_start_line,
                    character: form_binding.name_start_col,
                },
                end: Position {
                    line: form_binding.name_end_line,
                    character: form_binding.name_end_col,
                },
            },
            new_text: new_name.to_string(),
        };
        changes
            .entry(form_binding.uri.clone())
            .or_default()
            .push(def_edit);

        // フォーム名に対応するHTMLスコープ参照を収集して編集を追加
        let controllers = self.index.resolve_controllers_for_html(uri, form_binding.scope_start_line);
        for controller_name in controllers {
            let symbol_name = format!(
                "{}.$scope.{}",
                controller_name,
                form_binding.name
            );

            // HTML内の参照を取得
            for reference in self.index.get_html_references_for_symbol(&symbol_name) {
                // フォームスコープ内の参照のみを追加
                if reference.start_line >= form_binding.scope_start_line
                    && reference.start_line <= form_binding.scope_end_line
                {
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
                    changes
                        .entry(reference.uri.clone())
                        .or_default()
                        .push(edit);
                }
            }

            // JS内の$scope.formName参照も取得して編集を追加
            for reference in self.index.get_references(&symbol_name) {
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
                changes
                    .entry(reference.uri.clone())
                    .or_default()
                    .push(edit);
            }
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
        // まずローカル変数定義をチェック
        if let Some(local_var_def) = self.index.find_html_local_variable_definition_at(
            uri,
            position.line,
            position.character,
        ) {
            return Some(PrepareRenameResponse::Range(Range {
                start: Position {
                    line: local_var_def.name_start_line,
                    character: local_var_def.name_start_col,
                },
                end: Position {
                    line: local_var_def.name_end_line,
                    character: local_var_def.name_end_col,
                },
            }));
        }

        // ローカル変数参照をチェック
        if let Some(local_var_ref) = self.index.find_html_local_variable_at(
            uri,
            position.line,
            position.character,
        ) {
            return Some(PrepareRenameResponse::Range(Range {
                start: Position {
                    line: local_var_ref.start_line,
                    character: local_var_ref.start_col,
                },
                end: Position {
                    line: local_var_ref.end_line,
                    character: local_var_ref.end_col,
                },
            }));
        }

        // フォームバインディング定義をチェック
        if let Some(form_binding) = self.index.find_html_form_binding_at(
            uri,
            position.line,
            position.character,
        ) {
            return Some(PrepareRenameResponse::Range(Range {
                start: Position {
                    line: form_binding.name_start_line,
                    character: form_binding.name_start_col,
                },
                end: Position {
                    line: form_binding.name_end_line,
                    character: form_binding.name_end_col,
                },
            }));
        }

        // HTMLスコープ参照を取得
        let html_ref = self.index.find_html_scope_reference_at(
            uri,
            position.line,
            position.character,
        )?;

        let base_name = html_ref.property_path.split('.').next().unwrap_or(&html_ref.property_path);

        // フォームバインディング参照かどうかをチェック
        if let Some(form_binding) = self.index.find_form_binding_definition(uri, base_name, position.line) {
            // フォームバインディングの場合、定義位置の範囲を返す
            return Some(PrepareRenameResponse::Range(Range {
                start: Position {
                    line: form_binding.name_start_line,
                    character: form_binding.name_start_col,
                },
                end: Position {
                    line: form_binding.name_end_line,
                    character: form_binding.name_end_col,
                },
            }));
        }

        // 継承されたローカル変数かどうかをチェック
        if let Some(var_def) = self.index.find_local_variable_definition(uri, base_name, position.line) {
            // 継承されたローカル変数の場合、定義位置の範囲を返す
            return Some(PrepareRenameResponse::Range(Range {
                start: Position {
                    line: var_def.name_start_line,
                    character: var_def.name_start_col,
                },
                end: Position {
                    line: var_def.name_end_line,
                    character: var_def.name_end_col,
                },
            }));
        }

        // 通常のスコープ参照の範囲を返す
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
        for reference in self.index.get_all_references(symbol_name) {
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
