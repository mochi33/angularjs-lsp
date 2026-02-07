use std::sync::Arc;

use tower_lsp::lsp_types::*;
use tracing::debug;

use crate::index::Index;
use crate::model::SymbolKind;
use crate::util::is_html_file;

pub struct DefinitionHandler {
    index: Arc<Index>,
}

impl DefinitionHandler {
    pub fn new(index: Arc<Index>) -> Self {
        Self { index }
    }

    pub fn goto_definition(&self, params: GotoDefinitionParams) -> Option<GotoDefinitionResponse> {
        self.goto_definition_with_source(params, None)
    }

    pub fn goto_definition_with_source(
        &self,
        params: GotoDefinitionParams,
        source: Option<&str>,
    ) -> Option<GotoDefinitionResponse> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        // HTMLファイルの場合は専用の処理
        if is_html_file(&uri) {
            return self.goto_definition_from_html(&uri, position, source);
        }

        let symbol_name = self.index.definitions.find_symbol_at_position(
            &uri,
            position.line,
            position.character,
        )?;

        let definitions = self.index.definitions.get_definitions(&symbol_name);

        if definitions.is_empty() {
            return None;
        }

        let locations: Vec<Location> = definitions
            .into_iter()
            .map(|def| Location {
                uri: def.uri.clone(),
                range: def.definition_span.to_lsp_range(),
            })
            .collect();

        Some(GotoDefinitionResponse::Array(locations))
    }

    /// HTMLファイルからの定義ジャンプ
    fn goto_definition_from_html(
        &self,
        uri: &Url,
        position: Position,
        source: Option<&str>,
    ) -> Option<GotoDefinitionResponse> {
        // 0. まずカスタムディレクティブ/コンポーネント参照をチェック
        if let Some(directive_ref) =
            self.index
                .html
                .find_html_directive_reference_at(uri, position.line, position.character)
        {
            // ディレクティブまたはコンポーネント定義を検索
            let definitions = self
                .index
                .definitions
                .get_definitions(&directive_ref.directive_name);
            let directive_defs: Vec<_> = definitions
                .into_iter()
                .filter(|d| d.kind == SymbolKind::Directive || d.kind == SymbolKind::Component)
                .collect();

            if !directive_defs.is_empty() {
                let locations: Vec<Location> = directive_defs
                    .into_iter()
                    .map(|def| Location {
                        uri: def.uri.clone(),
                        range: def.definition_span.to_lsp_range(),
                    })
                    .collect();

                return Some(GotoDefinitionResponse::Array(locations));
            }
        }

        // 1. まずローカル変数をチェック（定義位置にカーソルがある場合）
        if let Some(local_var_def) = self.index.html.find_html_local_variable_definition_at(
            uri,
            position.line,
            position.character,
        ) {
            return Some(GotoDefinitionResponse::Scalar(Location {
                uri: local_var_def.uri.clone(),
                range: local_var_def.name_span().to_lsp_range(),
            }));
        }

        // 2. ローカル変数参照をチェック
        if let Some(local_var_ref) = self.index.html.find_html_local_variable_at(
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
                    range: var_def.name_span().to_lsp_range(),
                }));
            }
        }

        // 3. フォームバインディングをチェック（定義位置にカーソルがある場合）
        if let Some(form_binding) = self.index.html.find_html_form_binding_at(
            uri,
            position.line,
            position.character,
        ) {
            return Some(GotoDefinitionResponse::Scalar(Location {
                uri: form_binding.uri.clone(),
                range: form_binding.name_span().to_lsp_range(),
            }));
        }

        // 4. 位置からHTMLスコープ参照を取得
        let html_ref =
            self.index
                .html
                .find_html_scope_reference_at(uri, position.line, position.character);

        // スコープ参照が見つからない場合、継承されたローカル変数またはフォームバインディングを探す
        // (子テンプレートが親より先に解析された場合に発生)
        if html_ref.is_none() {
            if let Some(src) = source {
                if let Some(identifier) = self.extract_identifier_at_position(src, position) {
                    debug!(
                        "goto_definition_from_html: no scope ref, trying inherited local var or form binding '{}'",
                        identifier
                    );

                    // まず継承されたフォームバインディングをチェック
                    let base_name = identifier.split('.').next().unwrap_or(&identifier);
                    if let Some(form_binding) =
                        self.index
                            .find_form_binding_definition(uri, base_name, position.line)
                    {
                        return Some(GotoDefinitionResponse::Scalar(Location {
                            uri: form_binding.uri.clone(),
                            range: form_binding.name_span().to_lsp_range(),
                        }));
                    }

                    // 次に継承されたローカル変数をチェック
                    if let Some(var_def) =
                        self.index
                            .find_local_variable_definition(uri, &identifier, position.line)
                    {
                        return Some(GotoDefinitionResponse::Scalar(Location {
                            uri: var_def.uri.clone(),
                            range: var_def.name_span().to_lsp_range(),
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
        let base_name = html_ref
            .property_path
            .split('.')
            .next()
            .unwrap_or(&html_ref.property_path);
        if let Some(form_binding) =
            self.index
                .find_form_binding_definition(uri, base_name, position.line)
        {
            debug!(
                "goto_definition_from_html: '{}' is a form binding",
                base_name
            );
            return Some(GotoDefinitionResponse::Scalar(Location {
                uri: form_binding.uri.clone(),
                range: form_binding.name_span().to_lsp_range(),
            }));
        }

        // 5b. 継承されたローカル変数かどうかをチェック
        if let Some(var_def) =
            self.index
                .find_local_variable_definition(uri, base_name, position.line)
        {
            debug!(
                "goto_definition_from_html: '{}' is an inherited local variable",
                base_name
            );
            return Some(GotoDefinitionResponse::Scalar(Location {
                uri: var_def.uri.clone(),
                range: var_def.name_span().to_lsp_range(),
            }));
        }

        // 5c. alias.property 形式かチェック（controller as alias 構文）
        let (resolved_controller, property_path) = if html_ref.property_path.contains('.') {
            let parts: Vec<&str> = html_ref.property_path.splitn(2, '.').collect();
            if parts.len() == 2 {
                let alias = parts[0];
                let prop = parts[1];
                if let Some(controller) =
                    self.index
                        .resolve_controller_by_alias(uri, position.line, alias)
                {
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

            let symbol_name = format!("{}.$scope.{}", controller_name, property_path);

            debug!(
                "goto_definition_from_html: looking up symbol '{}'",
                symbol_name
            );

            let definitions = self.index.definitions.get_definitions(&symbol_name);

            if !definitions.is_empty() {
                let locations: Vec<Location> = definitions
                    .into_iter()
                    .map(|def| Location {
                        uri: def.uri.clone(),
                        range: def.definition_span.to_lsp_range(),
                    })
                    .collect();

                debug!(
                    "goto_definition_from_html: found {} locations",
                    locations.len()
                );

                return Some(GotoDefinitionResponse::Array(locations));
            }
        }

        // 5. controller as 構文の場合、this.method パターンも検索
        if is_controller_as {
            for controller_name in &controllers {
                let symbol_name = format!("{}.{}", controller_name, property_path);

                debug!(
                    "goto_definition_from_html: looking up controller method '{}'",
                    symbol_name
                );

                let definitions = self.index.definitions.get_definitions(&symbol_name);

                if !definitions.is_empty() {
                    let locations: Vec<Location> = definitions
                        .into_iter()
                        .map(|def| Location {
                            uri: def.uri.clone(),
                            range: def.definition_span.to_lsp_range(),
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

        // 6. $rootScope からのグローバル参照を検索
        let root_scope_defs = self
            .index
            .definitions
            .find_root_scope_definitions_by_property(&property_path);
        if !root_scope_defs.is_empty() {
            debug!(
                "goto_definition_from_html: found {} $rootScope definitions for '{}'",
                root_scope_defs.len(),
                property_path
            );

            let locations: Vec<Location> = root_scope_defs
                .into_iter()
                .map(|def| Location {
                    uri: def.uri.clone(),
                    range: def.definition_span.to_lsp_range(),
                })
                .collect();

            return Some(GotoDefinitionResponse::Array(locations));
        }

        debug!(
            "goto_definition_from_html: no definitions found in any controller or $rootScope"
        );
        None
    }

    /// カーソル位置の識別子を抽出
    fn extract_identifier_at_position(
        &self,
        source: &str,
        position: Position,
    ) -> Option<String> {
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
