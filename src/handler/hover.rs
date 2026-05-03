use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::index::{HtmlResolution, Index};
use crate::model::{
    DirectiveUsageType, HtmlDirectiveReference, HtmlFormBinding, HtmlLocalVariable,
    HtmlLocalVariableSource, HtmlNgModelTarget, HtmlUiSrefReference, SymbolKind,
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
    ///
    /// 解決優先順位は [`Index::resolve_html_position`] に集約 (issue #49)。
    /// ここではその結果を `Hover` にマッピングするだけ。
    fn hover_from_html(&self, uri: &Url, position: Position) -> Option<Hover> {
        match self.index.resolve_html_position(uri, position, None)? {
            HtmlResolution::UiSref(r) => self.build_for_ui_sref(&r),
            HtmlResolution::Directive(r) => self.build_hover_for_directive(&r),
            HtmlResolution::LocalVarDef(v) | HtmlResolution::LocalVarRef(v) => {
                self.build_hover_for_local_variable(&v)
            }
            HtmlResolution::FormBindingDef(f) | HtmlResolution::InheritedFormBinding(f) => {
                self.build_hover_for_form_binding(&f)
            }
            HtmlResolution::InheritedLocalVar(v) => self.build_hover_for_local_variable(&v),
            HtmlResolution::Scope {
                controllers,
                property_path,
                is_alias,
            } => self.build_for_scope(uri, &controllers, &property_path, is_alias),
        }
    }

    fn build_for_ui_sref(&self, ui_sref: &HtmlUiSrefReference) -> Option<Hover> {
        let definitions = self.index.definitions.get_definitions(&ui_sref.state_name);
        let state_def = definitions
            .into_iter()
            .find(|d| d.kind == SymbolKind::UiRouterState)?;
        self.build_hover_for_symbol(&state_def.name)
    }

    /// `Scope` variant の後段チェイン:
    /// `{ctrl}.$scope.{prop}` → (alias なら) `{ctrl}.{prop}` → ng-model 暗黙的定義
    ///
    /// (definition / references と異なり hover では `$rootScope` ホバーは未対応 —
    /// 既存挙動を維持。必要なら別 Issue で追加)
    fn build_for_scope(
        &self,
        uri: &Url,
        controllers: &[String],
        property_path: &str,
        is_alias: bool,
    ) -> Option<Hover> {
        for controller_name in controllers {
            let symbol_name = format!("{}.$scope.{}", controller_name, property_path);
            if let Some(hover) = self.build_hover_for_symbol(&symbol_name) {
                return Some(hover);
            }
        }

        if is_alias {
            for controller_name in controllers {
                let symbol_name = format!("{}.{}", controller_name, property_path);
                if let Some(hover) = self.build_hover_for_symbol(&symbol_name) {
                    return Some(hover);
                }
            }
        }

        for controller_name in controllers {
            if let Some(target) =
                self.index
                    .find_ng_model_implicit_def_target(uri, controller_name, property_path)
            {
                return self.build_hover_for_ng_model_target(&target, controller_name);
            }
        }

        None
    }

    /// ng-model 暗黙的定義用のホバー情報を構築
    fn build_hover_for_ng_model_target(
        &self,
        target: &HtmlNgModelTarget,
        controller_name: &str,
    ) -> Option<Hover> {
        let file_name = target
            .uri
            .to_file_path()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| target.uri.to_string());

        let content = format!(
            "**{}** (*ng-model implicit `$scope` property*)\n\n\
            Bound via `ng-model` (`$scope` of `{}`).\n\n\
            Defined at: `{}:{}`\n\n\
            ---\n\n\
            AngularJS auto-creates this property on `$scope` when the binding fires. \
            For clarity, consider initializing it explicitly in the controller.",
            target.property_path,
            controller_name,
            file_name,
            target.start_line + 1,
        );

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: None,
        })
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
            HtmlLocalVariableSource::NgRepeatSpecial => "ng-repeat special",
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
