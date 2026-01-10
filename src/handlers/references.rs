use std::sync::Arc;

use tower_lsp::lsp_types::*;
use tracing::debug;

use crate::index::{HtmlFormBinding, HtmlLocalVariable, SymbolIndex};

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
        // 1. まずローカル変数をチェック（定義位置にカーソルがある場合）
        if let Some(local_var_def) = self.index.find_html_local_variable_definition_at(
            uri,
            position.line,
            position.character,
        ) {
            return self.collect_local_variable_references(&local_var_def, include_declaration);
        }

        // 2. ローカル変数参照をチェック
        if let Some(local_var_ref) = self.index.find_html_local_variable_at(
            uri,
            position.line,
            position.character,
        ) {
            if let Some(var_def) = self.index.find_local_variable_definition(
                uri,
                &local_var_ref.variable_name,
                position.line,
            ) {
                return self.collect_local_variable_references(&var_def, include_declaration);
            }
        }

        // 3. フォームバインディングをチェック（定義位置にカーソルがある場合）
        if let Some(form_binding) = self.index.find_html_form_binding_at(
            uri,
            position.line,
            position.character,
        ) {
            return self.collect_form_binding_references(&form_binding, include_declaration);
        }

        // 5. 位置からHTMLスコープ参照を取得
        let html_ref = self.index.find_html_scope_reference_at(
            uri,
            position.line,
            position.character,
        )?;

        debug!(
            "find_references_from_html: found reference '{}' at {}:{}",
            html_ref.property_path, position.line, position.character
        );

        // 5a. フォームバインディング参照かどうかをチェック
        let base_name = html_ref.property_path.split('.').next().unwrap_or(&html_ref.property_path);
        if let Some(form_binding) = self.index.find_form_binding_definition(uri, base_name, position.line) {
            debug!(
                "find_references_from_html: '{}' is a form binding",
                base_name
            );
            return self.collect_form_binding_references(&form_binding, include_declaration);
        }

        // 5b. 継承されたローカル変数かどうかをチェック
        // スコープ参照として登録されていても、実際は継承されたローカル変数の可能性がある
        let base_name = html_ref.property_path.split('.').next().unwrap_or(&html_ref.property_path);
        if let Some(var_def) = self.index.find_local_variable_definition(uri, base_name, position.line) {
            debug!(
                "find_references_from_html: '{}' is an inherited local variable",
                base_name
            );
            return self.collect_local_variable_references(&var_def, include_declaration);
        }

        // 3b. alias.property形式かチェック（controller as alias構文）
        let (resolved_controller, property_path) = if html_ref.property_path.contains('.') {
            let parts: Vec<&str> = html_ref.property_path.splitn(2, '.').collect();
            if parts.len() == 2 {
                let alias = parts[0];
                let prop = parts[1];
                // aliasからコントローラーを解決
                if let Some(controller) = self.index.resolve_controller_by_alias(uri, position.line, alias) {
                    debug!(
                        "find_references_from_html: resolved alias '{}' to controller '{}'",
                        alias, controller
                    );
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
        let is_controller_as = resolved_controller.is_some();
        let controllers = if let Some(controller) = resolved_controller {
            vec![controller]
        } else {
            self.index.resolve_controllers_for_html(uri, position.line)
        };

        // 4. 各コントローラーを順番に試して、定義が見つかったものを返す
        for controller_name in &controllers {
            debug!(
                "find_references_from_html: trying controller '{}'",
                controller_name
            );

            // $scope.property形式で検索
            let symbol_name = format!(
                "{}.$scope.{}",
                controller_name,
                property_path
            );

            debug!(
                "find_references_from_html: looking up symbol '{}'",
                symbol_name
            );

            if let Some(locations) = self.collect_references(&symbol_name, include_declaration) {
                return Some(locations);
            }
        }

        // 5. controller as構文の場合、this.methodパターンも検索
        // ControllerName.method形式で登録されている
        if is_controller_as {
            for controller_name in &controllers {
                let symbol_name = format!(
                    "{}.{}",
                    controller_name,
                    property_path
                );

                debug!(
                    "find_references_from_html: looking up controller method '{}'",
                    symbol_name
                );

                if let Some(locations) = self.collect_references(&symbol_name, include_declaration) {
                    return Some(locations);
                }
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

    /// ローカル変数の定義と参照を収集
    fn collect_local_variable_references(
        &self,
        var_def: &HtmlLocalVariable,
        include_declaration: bool,
    ) -> Option<Vec<Location>> {
        let mut locations = Vec::new();

        // 定義位置を追加
        if include_declaration {
            locations.push(Location {
                uri: var_def.uri.clone(),
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
            });
        }

        // スコープ内の全参照を追加（定義されたファイル内）
        let refs = self.index.get_local_variable_references(
            &var_def.uri,
            &var_def.name,
            var_def.scope_start_line,
            var_def.scope_end_line,
        );

        for reference in refs {
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

        // 継承先の子テンプレート内の参照も追加（ng-include経由）
        let inherited_refs = self.index.get_inherited_local_variable_references(
            &var_def.uri,
            &var_def.name,
        );

        for reference in inherited_refs {
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

    /// フォームバインディングの定義と参照を収集
    fn collect_form_binding_references(
        &self,
        form_binding: &HtmlFormBinding,
        include_declaration: bool,
    ) -> Option<Vec<Location>> {
        let mut locations = Vec::new();

        // 定義位置を追加（<form name="x">のname属性値）
        if include_declaration {
            locations.push(Location {
                uri: form_binding.uri.clone(),
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
            });
        }

        // フォーム名に対応するHTMLスコープ参照を収集
        // フォーム名はスコーププロパティとして参照されるため、
        // $scope.formName形式ではなく、単純なformName参照を探す
        // ただし、コントローラースコープ内の参照のみを対象とする
        let controllers = self.index.resolve_controllers_for_html(&form_binding.uri, form_binding.scope_start_line);
        for controller_name in controllers {
            // ControllerName.$scope.formName形式でシンボル名を構築
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
            }

            // JS内の$scope.formName参照も取得
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
        }

        if locations.is_empty() {
            None
        } else {
            Some(locations)
        }
    }

    pub fn goto_definition(&self, params: GotoDefinitionParams) -> Option<GotoDefinitionResponse> {
        self.goto_definition_with_source(params, None)
    }

    pub fn goto_definition_with_source(&self, params: GotoDefinitionParams, source: Option<&str>) -> Option<GotoDefinitionResponse> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        // HTMLファイルの場合は専用の処理
        if Self::is_html_file(&uri) {
            return self.goto_definition_from_html(&uri, position, source);
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
    fn goto_definition_from_html(&self, uri: &Url, position: Position, source: Option<&str>) -> Option<GotoDefinitionResponse> {
        // 1. まずローカル変数をチェック（定義位置にカーソルがある場合）
        if let Some(local_var_def) = self.index.find_html_local_variable_definition_at(
            uri,
            position.line,
            position.character,
        ) {
            return Some(GotoDefinitionResponse::Scalar(Location {
                uri: local_var_def.uri.clone(),
                range: Range {
                    start: Position {
                        line: local_var_def.name_start_line,
                        character: local_var_def.name_start_col,
                    },
                    end: Position {
                        line: local_var_def.name_end_line,
                        character: local_var_def.name_end_col,
                    },
                },
            }));
        }

        // 2. ローカル変数参照をチェック
        if let Some(local_var_ref) = self.index.find_html_local_variable_at(
            uri,
            position.line,
            position.character,
        ) {
            if let Some(var_def) = self.index.find_local_variable_definition(
                uri,
                &local_var_ref.variable_name,
                position.line,
            ) {
                return Some(GotoDefinitionResponse::Scalar(Location {
                    uri: var_def.uri.clone(),
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
                }));
            }
        }

        // 3. フォームバインディングをチェック（定義位置にカーソルがある場合）
        if let Some(form_binding) = self.index.find_html_form_binding_at(
            uri,
            position.line,
            position.character,
        ) {
            return Some(GotoDefinitionResponse::Scalar(Location {
                uri: form_binding.uri.clone(),
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
            }));
        }

        // 4. 位置からHTMLスコープ参照を取得
        let html_ref = self.index.find_html_scope_reference_at(
            uri,
            position.line,
            position.character,
        );

        // スコープ参照が見つからない場合、継承されたローカル変数を探す
        // (子テンプレートが親より先に解析された場合に発生)
        if html_ref.is_none() {
            if let Some(src) = source {
                if let Some(identifier) = self.extract_identifier_at_position(src, position) {
                    debug!(
                        "goto_definition_from_html: no scope ref, trying inherited local var '{}'",
                        identifier
                    );
                    if let Some(var_def) = self.index.find_local_variable_definition(
                        uri,
                        &identifier,
                        position.line,
                    ) {
                        return Some(GotoDefinitionResponse::Scalar(Location {
                            uri: var_def.uri.clone(),
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
                        }));
                    }
                }
            }
            return None;
        }

        let html_ref = html_ref.unwrap();

        debug!(
            "goto_definition_from_html: found reference '{}' at {}:{}",
            html_ref.property_path, position.line, position.character
        );

        // 5a. フォームバインディング参照かどうかをチェック
        let base_name = html_ref.property_path.split('.').next().unwrap_or(&html_ref.property_path);
        if let Some(form_binding) = self.index.find_form_binding_definition(uri, base_name, position.line) {
            debug!(
                "goto_definition_from_html: '{}' is a form binding",
                base_name
            );
            return Some(GotoDefinitionResponse::Scalar(Location {
                uri: form_binding.uri.clone(),
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
            }));
        }

        // 5b. 継承されたローカル変数かどうかをチェック
        // スコープ参照として登録されていても、実際は継承されたローカル変数の可能性がある
        // (子テンプレートが親より先に解析された場合に発生)
        if let Some(var_def) = self.index.find_local_variable_definition(uri, base_name, position.line) {
            debug!(
                "goto_definition_from_html: '{}' is an inherited local variable",
                base_name
            );
            return Some(GotoDefinitionResponse::Scalar(Location {
                uri: var_def.uri.clone(),
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
            }));
        }

        // 5c. alias.property形式かチェック（controller as alias構文）
        let (resolved_controller, property_path) = if html_ref.property_path.contains('.') {
            let parts: Vec<&str> = html_ref.property_path.splitn(2, '.').collect();
            if parts.len() == 2 {
                let alias = parts[0];
                let prop = parts[1];
                // aliasからコントローラーを解決
                if let Some(controller) = self.index.resolve_controller_by_alias(uri, position.line, alias) {
                    debug!(
                        "goto_definition_from_html: resolved alias '{}' to controller '{}'",
                        alias, controller
                    );
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
        let is_controller_as = resolved_controller.is_some();
        let controllers = if let Some(controller) = resolved_controller {
            vec![controller]
        } else {
            self.index.resolve_controllers_for_html(uri, position.line)
        };

        // 4. 各コントローラーを順番に試して、定義が見つかったものを返す
        for controller_name in &controllers {
            debug!(
                "goto_definition_from_html: trying controller '{}'",
                controller_name
            );

            let symbol_name = format!(
                "{}.$scope.{}",
                controller_name,
                property_path
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

        // 5. controller as構文の場合、this.methodパターンも検索
        // ControllerName.method形式で登録されている
        if is_controller_as {
            for controller_name in &controllers {
                let symbol_name = format!(
                    "{}.{}",
                    controller_name,
                    property_path
                );

                debug!(
                    "goto_definition_from_html: looking up controller method '{}'",
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
                        "goto_definition_from_html: found {} controller method locations",
                        locations.len()
                    );

                    return Some(GotoDefinitionResponse::Array(locations));
                }
            }
        }

        debug!("goto_definition_from_html: no definitions found in any controller");
        None
    }

    /// カーソル位置の識別子を抽出
    fn extract_identifier_at_position(&self, source: &str, position: Position) -> Option<String> {
        let lines: Vec<&str> = source.lines().collect();
        let line = lines.get(position.line as usize)?;
        let col = position.character as usize;

        if col >= line.len() {
            return None;
        }

        // カーソル位置から識別子の開始と終了を探す
        let chars: Vec<char> = line.chars().collect();

        // 開始位置を探す
        let mut start = col;
        while start > 0 {
            let c = chars[start - 1];
            if !c.is_alphanumeric() && c != '_' && c != '$' {
                break;
            }
            start -= 1;
        }

        // 終了位置を探す
        let mut end = col;
        while end < chars.len() {
            let c = chars[end];
            if !c.is_alphanumeric() && c != '_' && c != '$' {
                break;
            }
            end += 1;
        }

        if start == end {
            return None;
        }

        let identifier: String = chars[start..end].iter().collect();
        if identifier.is_empty() {
            None
        } else {
            Some(identifier)
        }
    }
}
