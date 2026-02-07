use std::sync::Arc;

use tower_lsp::lsp_types::*;
use tracing::debug;

use crate::index::Index;
use crate::model::{HtmlFormBinding, HtmlLocalVariable, SymbolKind};
use crate::util::is_html_file;

pub struct ReferencesHandler {
    index: Arc<Index>,
}

impl ReferencesHandler {
    pub fn new(index: Arc<Index>) -> Self {
        Self { index }
    }

    pub fn find_references(&self, params: ReferenceParams) -> Option<Vec<Location>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let include_declaration = params.context.include_declaration;

        // HTMLファイルの場合は専用の処理
        if is_html_file(&uri) {
            return self.find_references_from_html(&uri, position, include_declaration);
        }

        let symbol_name = self.index.definitions.find_symbol_at_position(
            &uri,
            position.line,
            position.character,
        )?;

        // シンボルがディレクティブまたはコンポーネントの場合、HTML参照も収集
        let definitions = self.index.definitions.get_definitions(&symbol_name);
        if definitions
            .iter()
            .any(|d| d.kind == SymbolKind::Directive || d.kind == SymbolKind::Component)
        {
            return self.collect_directive_all_references(&symbol_name, include_declaration);
        }

        self.collect_references(&symbol_name, include_declaration)
    }

    /// HTMLファイルからの参照検索
    fn find_references_from_html(
        &self,
        uri: &Url,
        position: Position,
        include_declaration: bool,
    ) -> Option<Vec<Location>> {
        // 0. まずカスタムディレクティブ参照をチェック
        if let Some(directive_ref) = self.index.html.find_html_directive_reference_at(
            uri,
            position.line,
            position.character,
        ) {
            return self.collect_directive_all_references(
                &directive_ref.directive_name,
                include_declaration,
            );
        }

        // 1. まずローカル変数をチェック（定義位置にカーソルがある場合）
        if let Some(local_var_def) = self.index.html.find_html_local_variable_definition_at(
            uri,
            position.line,
            position.character,
        ) {
            return self.collect_local_variable_references(&local_var_def, include_declaration);
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
                return self.collect_local_variable_references(&var_def, include_declaration);
            }
        }

        // 3. フォームバインディングをチェック（定義位置にカーソルがある場合）
        if let Some(form_binding) = self.index.html.find_html_form_binding_at(
            uri,
            position.line,
            position.character,
        ) {
            return self.collect_form_binding_references(&form_binding, include_declaration);
        }

        // 5. 位置からHTMLスコープ参照を取得
        let html_ref = self.index.html.find_html_scope_reference_at(
            uri,
            position.line,
            position.character,
        )?;

        debug!(
            "find_references_from_html: found reference '{}' at {}:{}",
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
                "find_references_from_html: '{}' is a form binding",
                base_name
            );
            return self.collect_form_binding_references(&form_binding, include_declaration);
        }

        // 5b. 継承されたローカル変数かどうかをチェック
        let base_name = html_ref
            .property_path
            .split('.')
            .next()
            .unwrap_or(&html_ref.property_path);
        if let Some(var_def) =
            self.index
                .find_local_variable_definition(uri, base_name, position.line)
        {
            debug!(
                "find_references_from_html: '{}' is an inherited local variable",
                base_name
            );
            return self.collect_local_variable_references(&var_def, include_declaration);
        }

        // 3b. alias.property 形式かチェック（controller as alias 構文）
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

            let symbol_name = format!("{}.$scope.{}", controller_name, property_path);

            debug!(
                "find_references_from_html: looking up symbol '{}'",
                symbol_name
            );

            if self.index.definitions.has_definition(&symbol_name) {
                if let Some(locations) =
                    self.collect_references(&symbol_name, include_declaration)
                {
                    return Some(locations);
                }
            }
        }

        // 5. controller as 構文の場合、this.method パターンも検索
        if is_controller_as {
            for controller_name in &controllers {
                let symbol_name = format!("{}.{}", controller_name, property_path);

                debug!(
                    "find_references_from_html: looking up controller method '{}'",
                    symbol_name
                );

                if self.index.definitions.has_definition(&symbol_name) {
                    if let Some(locations) =
                        self.collect_references(&symbol_name, include_declaration)
                    {
                        return Some(locations);
                    }
                }
            }
        }

        // 6. $rootScope からのグローバル参照を検索
        if let Some(root_scope_symbol) = self
            .index
            .definitions
            .find_root_scope_symbol_name_by_property(&property_path)
        {
            debug!(
                "find_references_from_html: found $rootScope symbol '{}'",
                root_scope_symbol
            );
            return self.collect_references(&root_scope_symbol, include_declaration);
        }

        None
    }

    /// シンボル名から定義と参照を収集
    fn collect_references(
        &self,
        symbol_name: &str,
        include_declaration: bool,
    ) -> Option<Vec<Location>> {
        let mut locations = Vec::new();

        // Add definition locations if requested
        if include_declaration {
            for def in self.index.definitions.get_definitions(symbol_name) {
                locations.push(Location {
                    uri: def.uri.clone(),
                    range: def.definition_span.to_lsp_range(),
                });
            }
        }

        // Add reference locations (JS + HTML)
        for reference in self.index.get_all_references(symbol_name) {
            locations.push(Location {
                uri: reference.uri.clone(),
                range: reference.span.to_lsp_range(),
            });
        }

        if locations.is_empty() {
            None
        } else {
            Some(locations)
        }
    }

    /// ディレクティブの定義と全参照を収集
    fn collect_directive_all_references(
        &self,
        directive_name: &str,
        include_declaration: bool,
    ) -> Option<Vec<Location>> {
        let mut locations = Vec::new();

        // 定義位置を追加（SymbolKind::Directive または SymbolKind::Component）
        if include_declaration {
            for def in self.index.definitions.get_definitions(directive_name) {
                if def.kind == SymbolKind::Directive || def.kind == SymbolKind::Component {
                    locations.push(Location {
                        uri: def.uri.clone(),
                        range: def.definition_span.to_lsp_range(),
                    });
                }
            }
        }

        // HTML内の参照を追加
        for reference in self
            .index
            .html
            .get_html_directive_references(directive_name)
        {
            locations.push(Location {
                uri: reference.uri.clone(),
                range: reference.span().to_lsp_range(),
            });
        }

        // JS内の参照も追加
        for reference in self.index.get_all_references(directive_name) {
            locations.push(Location {
                uri: reference.uri.clone(),
                range: reference.span.to_lsp_range(),
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
                range: var_def.name_span().to_lsp_range(),
            });
        }

        // スコープ内の全参照を追加（定義されたファイル内）
        let refs = self.index.html.get_local_variable_references(
            &var_def.uri,
            &var_def.name,
            var_def.scope_start_line,
            var_def.scope_end_line,
        );

        for reference in refs {
            locations.push(Location {
                uri: reference.uri.clone(),
                range: reference.span().to_lsp_range(),
            });
        }

        // 継承先の子テンプレート内の参照も追加（ng-include経由）
        let inherited_refs =
            self.index
                .templates
                .get_inherited_local_variable_references(
                    &var_def.uri,
                    &var_def.name,
                    self.index.html.html_local_variable_references_raw(),
                );

        for reference in inherited_refs {
            locations.push(Location {
                uri: reference.uri.clone(),
                range: reference.span().to_lsp_range(),
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
                range: form_binding.name_span().to_lsp_range(),
            });
        }

        // フォーム名に対応するHTMLスコープ参照を収集
        let controllers = self
            .index
            .resolve_controllers_for_html(&form_binding.uri, form_binding.scope_start_line);
        for controller_name in controllers {
            let symbol_name = format!("{}.$scope.{}", controller_name, form_binding.name);

            // HTML内の参照を取得
            for reference in self.index.get_html_references_for_symbol(&symbol_name) {
                // フォームスコープ内の参照のみを追加
                if reference.span.start_line >= form_binding.scope_start_line
                    && reference.span.start_line <= form_binding.scope_end_line
                {
                    locations.push(Location {
                        uri: reference.uri.clone(),
                        range: reference.span.to_lsp_range(),
                    });
                }
            }

            // JS内の$scope.formName参照も取得
            for reference in self.index.definitions.get_references(&symbol_name) {
                locations.push(Location {
                    uri: reference.uri.clone(),
                    range: reference.span.to_lsp_range(),
                });
            }
        }

        if locations.is_empty() {
            None
        } else {
            Some(locations)
        }
    }
}
