use std::collections::HashMap;
use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::index::Index;
use crate::model::{HtmlFormBinding, HtmlLocalVariable};
use crate::util::is_html_file;

pub struct RenameHandler {
    index: Arc<Index>,
}

impl RenameHandler {
    pub fn new(index: Arc<Index>) -> Self {
        Self { index }
    }

    pub fn rename(&self, params: RenameParams) -> Option<WorkspaceEdit> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_name = params.new_name;

        // HTMLファイルの場合は専用の処理
        if is_html_file(&uri) {
            // まずローカル変数をチェック（定義位置にカーソルがある場合）
            if let Some(local_var_def) = self.index.html.find_html_local_variable_definition_at(
                &uri,
                position.line,
                position.character,
            ) {
                return self.collect_local_variable_edits(&local_var_def, &new_name);
            }

            // ローカル変数参照をチェック
            if let Some(local_var_ref) = self.index.html.find_html_local_variable_at(
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
            if let Some(form_binding) = self.index.html.find_html_form_binding_at(
                &uri,
                position.line,
                position.character,
            ) {
                return self.collect_form_binding_edits(&uri, &form_binding, &new_name);
            }

            // 3. スコープ参照をチェック（継承されたローカル変数やフォームバインディングの可能性もある）
            if let Some(html_ref) = self.index.html.find_html_scope_reference_at(
                &uri,
                position.line,
                position.character,
            ) {
                let base_name = html_ref
                    .property_path
                    .split('.')
                    .next()
                    .unwrap_or(&html_ref.property_path);

                // フォームバインディング参照かどうかをチェック
                if let Some(form_binding) =
                    self.index
                        .find_form_binding_definition(&uri, base_name, position.line)
                {
                    return self.collect_form_binding_edits(&uri, &form_binding, &new_name);
                }

                // 継承されたローカル変数かどうかをチェック
                if let Some(var_def) =
                    self.index
                        .find_local_variable_definition(&uri, base_name, position.line)
                {
                    return self.collect_local_variable_edits(&var_def, &new_name);
                }
            }

            // 通常のスコープ参照
            let symbol_name = self.resolve_symbol_name_from_html(&uri, position)?;
            return self.collect_edits(&symbol_name, &new_name);
        }

        let symbol_name = self.index.definitions.find_symbol_at_position(
            &uri,
            position.line,
            position.character,
        )?;

        self.collect_edits(&symbol_name, &new_name)
    }

    /// HTMLファイルからシンボル名を解決
    fn resolve_symbol_name_from_html(&self, uri: &Url, position: Position) -> Option<String> {
        // 1. 位置からHTMLスコープ参照を取得
        let html_ref = self.index.html.find_html_scope_reference_at(
            uri,
            position.line,
            position.character,
        )?;

        // 2. alias.property 形式かチェック（controller as alias 構文）
        let (resolved_controller, property_path) = if html_ref.property_path.contains('.') {
            let parts: Vec<&str> = html_ref.property_path.splitn(2, '.').collect();
            if parts.len() == 2 {
                let alias = parts[0];
                let prop = parts[1];
                if let Some(controller) =
                    self.index
                        .resolve_controller_by_alias(uri, position.line, alias)
                {
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
            self.index
                .resolve_controller_for_html(uri, position.line)?
        };

        // 4. シンボル名を構築 "ControllerName.$scope.property"
        let scope_symbol = format!("{}.$scope.{}", controller_name, property_path);

        // まず$scope形式で定義が見つかるか確認
        if !self
            .index
            .definitions
            .get_definitions(&scope_symbol)
            .is_empty()
        {
            return Some(scope_symbol);
        }

        // 5. controller as 構文の場合、ControllerName.method 形式も試す
        if resolved_controller.is_some() {
            let method_symbol = format!("{}.{}", controller_name, property_path);
            if !self
                .index
                .definitions
                .get_definitions(&method_symbol)
                .is_empty()
            {
                return Some(method_symbol);
            }
        }

        // デフォルトで$scope形式を返す
        Some(scope_symbol)
    }

    /// シンボル名から編集を収集
    fn collect_edits(&self, symbol_name: &str, new_name: &str) -> Option<WorkspaceEdit> {
        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();

        // Collect definition locations (use name_span for accurate renaming)
        for def in self.index.definitions.get_definitions(symbol_name) {
            let edit = TextEdit {
                range: def.name_span.to_lsp_range(),
                new_text: new_name.to_string(),
            };
            changes.entry(def.uri.clone()).or_default().push(edit);
        }

        // Collect reference locations
        for reference in self.index.get_all_references(symbol_name) {
            let edit = TextEdit {
                range: reference.span.to_lsp_range(),
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

    /// ローカル変数の編集を収集
    fn collect_local_variable_edits(
        &self,
        var_def: &HtmlLocalVariable,
        new_name: &str,
    ) -> Option<WorkspaceEdit> {
        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();

        // 定義位置の編集を追加
        let def_edit = TextEdit {
            range: var_def.name_span().to_lsp_range(),
            new_text: new_name.to_string(),
        };
        changes
            .entry(var_def.uri.clone())
            .or_default()
            .push(def_edit);

        // スコープ内の全参照の編集を追加
        let refs = self.index.html.get_local_variable_references(
            &var_def.uri,
            &var_def.name,
            var_def.scope_start_line,
            var_def.scope_end_line,
        );

        for reference in refs {
            let edit = TextEdit {
                range: reference.span().to_lsp_range(),
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
            range: form_binding.name_span().to_lsp_range(),
            new_text: new_name.to_string(),
        };
        changes
            .entry(form_binding.uri.clone())
            .or_default()
            .push(def_edit);

        // フォーム名に対応するHTMLスコープ参照を収集して編集を追加
        let controllers = self
            .index
            .resolve_controllers_for_html(uri, form_binding.scope_start_line);
        for controller_name in controllers {
            let symbol_name = format!("{}.$scope.{}", controller_name, form_binding.name);

            // HTML内の参照を取得
            for reference in self.index.get_html_references_for_symbol(&symbol_name) {
                // フォームスコープ内の参照のみを追加
                if reference.span.start_line >= form_binding.scope_start_line
                    && reference.span.start_line <= form_binding.scope_end_line
                {
                    let edit = TextEdit {
                        range: reference.span.to_lsp_range(),
                        new_text: new_name.to_string(),
                    };
                    changes
                        .entry(reference.uri.clone())
                        .or_default()
                        .push(edit);
                }
            }

            // JS内の$scope.formName参照も取得して編集を追加
            for reference in self.index.definitions.get_references(&symbol_name) {
                let edit = TextEdit {
                    range: reference.span.to_lsp_range(),
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

    pub fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Option<PrepareRenameResponse> {
        let uri = params.text_document.uri;
        let position = params.position;

        // HTMLファイルの場合は専用の処理
        if is_html_file(&uri) {
            return self.prepare_rename_from_html(&uri, position);
        }

        let symbol_name = self.index.definitions.find_symbol_at_position(
            &uri,
            position.line,
            position.character,
        )?;

        self.find_symbol_range_at_position(&symbol_name, &uri, position)
    }

    /// HTMLファイルからのprepare_rename
    fn prepare_rename_from_html(
        &self,
        uri: &Url,
        position: Position,
    ) -> Option<PrepareRenameResponse> {
        // まずローカル変数定義をチェック
        if let Some(local_var_def) = self.index.html.find_html_local_variable_definition_at(
            uri,
            position.line,
            position.character,
        ) {
            return Some(PrepareRenameResponse::Range(
                local_var_def.name_span().to_lsp_range(),
            ));
        }

        // ローカル変数参照をチェック
        if let Some(local_var_ref) = self.index.html.find_html_local_variable_at(
            uri,
            position.line,
            position.character,
        ) {
            return Some(PrepareRenameResponse::Range(
                local_var_ref.span().to_lsp_range(),
            ));
        }

        // フォームバインディング定義をチェック
        if let Some(form_binding) = self.index.html.find_html_form_binding_at(
            uri,
            position.line,
            position.character,
        ) {
            return Some(PrepareRenameResponse::Range(
                form_binding.name_span().to_lsp_range(),
            ));
        }

        // HTMLスコープ参照を取得
        let html_ref = self.index.html.find_html_scope_reference_at(
            uri,
            position.line,
            position.character,
        )?;

        let base_name = html_ref
            .property_path
            .split('.')
            .next()
            .unwrap_or(&html_ref.property_path);

        // フォームバインディング参照かどうかをチェック
        if let Some(form_binding) =
            self.index
                .find_form_binding_definition(uri, base_name, position.line)
        {
            return Some(PrepareRenameResponse::Range(
                form_binding.name_span().to_lsp_range(),
            ));
        }

        // 継承されたローカル変数かどうかをチェック
        if let Some(var_def) =
            self.index
                .find_local_variable_definition(uri, base_name, position.line)
        {
            return Some(PrepareRenameResponse::Range(
                var_def.name_span().to_lsp_range(),
            ));
        }

        // 通常のスコープ参照の範囲を返す
        Some(PrepareRenameResponse::Range(
            html_ref.span().to_lsp_range(),
        ))
    }

    /// シンボルの範囲を見つける
    fn find_symbol_range_at_position(
        &self,
        symbol_name: &str,
        uri: &Url,
        position: Position,
    ) -> Option<PrepareRenameResponse> {
        // First check definitions
        for def in self.index.definitions.get_definitions(symbol_name) {
            if def.uri == *uri && def.name_span.contains_line(position.line) {
                let in_range = if def.name_span.start_line == def.name_span.end_line {
                    position.character >= def.name_span.start_col
                        && position.character <= def.name_span.end_col
                } else {
                    true
                };
                if in_range {
                    return Some(PrepareRenameResponse::Range(
                        def.name_span.to_lsp_range(),
                    ));
                }
            }
        }

        // Then check references
        for reference in self.index.get_all_references(symbol_name) {
            if reference.uri == *uri && reference.span.contains_line(position.line) {
                let in_range = if reference.span.start_line == reference.span.end_line {
                    position.character >= reference.span.start_col
                        && position.character <= reference.span.end_col
                } else {
                    true
                };
                if in_range {
                    return Some(PrepareRenameResponse::Range(
                        reference.span.to_lsp_range(),
                    ));
                }
            }
        }

        None
    }
}
