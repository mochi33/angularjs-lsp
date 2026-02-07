use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::index::Index;
use crate::model::{
    DirectiveUsageType, HtmlDirectiveReference, HtmlFormBinding, HtmlLocalVariable,
    HtmlLocalVariableSource,
};
use crate::util::is_html_file;

pub struct HoverHandler {
    index: Arc<Index>,
}

impl HoverHandler {
    pub fn new(index: Arc<Index>) -> Self {
        Self { index }
    }

    pub fn hover(&self, params: HoverParams) -> Option<Hover> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        // HTMLファイルの場合は専用の処理
        if is_html_file(&uri) {
            return self.hover_from_html(&uri, position);
        }

        let symbol_name = self.index.definitions.find_symbol_at_position(
            &uri,
            position.line,
            position.character,
        )?;

        self.build_hover_for_symbol(&symbol_name)
    }

    /// HTMLファイルからのホバー
    fn hover_from_html(&self, uri: &Url, position: Position) -> Option<Hover> {
        // 1. まずローカル変数をチェック（定義位置にカーソルがある場合）
        if let Some(local_var_def) = self.index.html.find_html_local_variable_definition_at(
            uri,
            position.line,
            position.character,
        ) {
            return self.build_hover_for_local_variable(&local_var_def);
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
                return self.build_hover_for_local_variable(&var_def);
            }
        }

        // 3. フォームバインディングをチェック（定義位置にカーソルがある場合）
        if let Some(form_binding) = self.index.html.find_html_form_binding_at(
            uri,
            position.line,
            position.character,
        ) {
            return self.build_hover_for_form_binding(&form_binding);
        }

        // 3.5. ディレクティブ参照をチェック
        if let Some(directive_ref) = self.index.html.find_html_directive_reference_at(
            uri,
            position.line,
            position.character,
        ) {
            return self.build_hover_for_directive(&directive_ref);
        }

        // 4. 位置からHTMLスコープ参照を取得
        let html_ref = self.index.html.find_html_scope_reference_at(
            uri,
            position.line,
            position.character,
        )?;

        // 4a. フォームバインディング参照かどうかをチェック
        let base_name = html_ref
            .property_path
            .split('.')
            .next()
            .unwrap_or(&html_ref.property_path);
        if let Some(form_binding) =
            self.index
                .find_form_binding_definition(uri, base_name, position.line)
        {
            return self.build_hover_for_form_binding(&form_binding);
        }

        // 4b. 継承されたローカル変数かどうかをチェック
        if let Some(var_def) =
            self.index
                .find_local_variable_definition(uri, base_name, position.line)
        {
            return self.build_hover_for_local_variable(&var_def);
        }

        // 4c. alias.property 形式かチェック（controller as alias 構文）
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
        let controllers = if let Some(ref controller) = resolved_controller {
            vec![controller.clone()]
        } else {
            self.index.resolve_controllers_for_html(uri, position.line)
        };

        // 4. 各コントローラーを順番に試して、定義が見つかったものを返す
        for controller_name in &controllers {
            let symbol_name = format!("{}.$scope.{}", controller_name, property_path);

            if let Some(hover) = self.build_hover_for_symbol(&symbol_name) {
                return Some(hover);
            }
        }

        // 5. controller as 構文の場合、this.method パターンも検索
        if resolved_controller.is_some() {
            for controller_name in &controllers {
                let symbol_name = format!("{}.{}", controller_name, property_path);

                if let Some(hover) = self.build_hover_for_symbol(&symbol_name) {
                    return Some(hover);
                }
            }
        }

        None
    }

    /// シンボル名からホバー情報を構築
    fn build_hover_for_symbol(&self, symbol_name: &str) -> Option<Hover> {
        let definitions = self.index.definitions.get_definitions(symbol_name);

        if definitions.is_empty() {
            return None;
        }

        let def = &definitions[0];
        let kind_str = def.kind.as_str();

        // Build hover content
        let file_name = def
            .uri
            .to_file_path()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| def.uri.to_string());

        let reference_count = self.index.get_all_references(symbol_name).len();

        let mut content = format!("**{}** (*{}*)\n\n", def.name, kind_str);

        if let Some(ref docs) = def.docs {
            content.push_str(docs);
            content.push_str("\n\n---\n\n");
        }

        content.push_str(&format!(
            "Defined in: `{}:{}`\n",
            file_name,
            def.definition_span.start_line + 1
        ));

        if reference_count > 0 {
            content.push_str(&format!("\nReferences: {}", reference_count));
        }

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: None,
        })
    }

    /// ローカル変数用のホバー情報を構築
    fn build_hover_for_local_variable(&self, var_def: &HtmlLocalVariable) -> Option<Hover> {
        let source_str = match var_def.source {
            HtmlLocalVariableSource::NgInit => "ng-init",
            HtmlLocalVariableSource::NgRepeatIterator => "ng-repeat iterator",
            HtmlLocalVariableSource::NgRepeatKeyValue => "ng-repeat key/value",
        };

        let reference_count = self
            .index
            .html
            .get_local_variable_references(
                &var_def.uri,
                &var_def.name,
                var_def.scope_start_line,
                var_def.scope_end_line,
            )
            .len();

        let content = format!(
            "**{}** (*HTML local variable*)\n\n\
            Source: `{}`\n\
            Scope: lines {}-{}\n\n\
            References: {}",
            var_def.name,
            source_str,
            var_def.scope_start_line + 1,
            var_def.scope_end_line + 1,
            reference_count
        );

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: None,
        })
    }

    /// フォームバインディング用のホバー情報を構築
    fn build_hover_for_form_binding(&self, form_binding: &HtmlFormBinding) -> Option<Hover> {
        let content = format!(
            "**{}** (*form binding*)\n\n\
            AngularJS automatically binds this form to `$scope.{}`\n\n\
            Scope: lines {}-{}",
            form_binding.name,
            form_binding.name,
            form_binding.scope_start_line + 1,
            form_binding.scope_end_line + 1,
        );

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: None,
        })
    }

    /// ディレクティブ参照用のホバー情報を構築
    fn build_hover_for_directive(
        &self,
        directive_ref: &HtmlDirectiveReference,
    ) -> Option<Hover> {
        // ディレクティブ定義を取得
        let definitions = self
            .index
            .definitions
            .get_definitions(&directive_ref.directive_name);

        if definitions.is_empty() {
            return None;
        }

        let def = &definitions[0];

        // ファイル名を取得
        let file_name = def
            .uri
            .to_file_path()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| def.uri.to_string());

        // HTML内の参照数を取得
        let html_references = self
            .index
            .html
            .get_html_directive_references(&directive_ref.directive_name);
        let reference_count = html_references.len();

        let usage_type = match directive_ref.usage_type {
            DirectiveUsageType::Element => "element",
            DirectiveUsageType::Attribute => "attribute",
        };

        let mut content = format!(
            "**{}** (*directive*)\n\nUsed as: `{}`\n\n",
            directive_ref.directive_name, usage_type
        );

        if let Some(ref docs) = def.docs {
            content.push_str(docs);
            content.push_str("\n\n---\n\n");
        }

        content.push_str(&format!(
            "Defined in: `{}:{}`\n",
            file_name,
            def.definition_span.start_line + 1
        ));

        if reference_count > 0 {
            content.push_str(&format!("\nHTML references: {}", reference_count));
        }

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: None,
        })
    }
}
