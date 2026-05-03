use std::sync::Arc;

use tower_lsp::lsp_types::*;
use tracing::debug;

use crate::index::{HtmlResolution, Index};
use crate::model::{HtmlDirectiveReference, HtmlUiSrefReference, SymbolKind};
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
    ///
    /// 解決優先順位は [`Index::resolve_html_position`] に集約 (issue #49)。
    /// ここではその結果を `GotoDefinitionResponse` にマッピングするだけ。
    fn goto_definition_from_html(
        &self,
        uri: &Url,
        position: Position,
        source: Option<&str>,
    ) -> Option<GotoDefinitionResponse> {
        match self.index.resolve_html_position(uri, position, source)? {
            HtmlResolution::UiSref(r) => self.build_for_ui_sref(&r),
            HtmlResolution::Directive(r) => self.build_for_directive(&r),
            HtmlResolution::LocalVarDef(v) | HtmlResolution::LocalVarRef(v) => {
                Some(scalar(&v.uri, v.name_span().to_lsp_range()))
            }
            HtmlResolution::FormBindingDef(f) | HtmlResolution::InheritedFormBinding(f) => {
                Some(scalar(&f.uri, f.name_span().to_lsp_range()))
            }
            HtmlResolution::InheritedLocalVar(v) => {
                Some(scalar(&v.uri, v.name_span().to_lsp_range()))
            }
            HtmlResolution::Scope {
                controllers,
                property_path,
                is_alias,
            } => self.build_for_scope(uri, &controllers, &property_path, is_alias),
        }
    }

    fn build_for_ui_sref(&self, ui_sref: &HtmlUiSrefReference) -> Option<GotoDefinitionResponse> {
        let definitions = self.index.definitions.get_definitions(&ui_sref.state_name);
        let state_defs: Vec<_> = definitions
            .into_iter()
            .filter(|d| d.kind == SymbolKind::UiRouterState)
            .collect();
        if state_defs.is_empty() {
            // state 定義が見つからなくても、他のシンボルとして解決すべきではない
            // (ui-sref の値は state 名なので controller 名等での解決は誤動作する)
            return None;
        }
        let locations: Vec<Location> = state_defs
            .into_iter()
            .map(|def| Location {
                uri: def.uri.clone(),
                range: def.name_span.to_lsp_range(),
            })
            .collect();
        Some(GotoDefinitionResponse::Array(locations))
    }

    fn build_for_directive(
        &self,
        directive_ref: &HtmlDirectiveReference,
    ) -> Option<GotoDefinitionResponse> {
        let definitions = self
            .index
            .definitions
            .get_definitions(&directive_ref.directive_name);
        let directive_defs: Vec<_> = definitions
            .into_iter()
            .filter(|d| d.kind == SymbolKind::Directive || d.kind == SymbolKind::Component)
            .collect();
        if directive_defs.is_empty() {
            return None;
        }
        let locations: Vec<Location> = directive_defs
            .into_iter()
            .map(|def| Location {
                uri: def.uri.clone(),
                range: def.definition_span.to_lsp_range(),
            })
            .collect();
        Some(GotoDefinitionResponse::Array(locations))
    }

    /// `Scope` variant の後段チェイン:
    /// `{ctrl}.$scope.{prop}` → (alias なら) `{ctrl}.{prop}` → `$rootScope.{prop}`
    /// → ng-model 暗黙的定義
    fn build_for_scope(
        &self,
        uri: &Url,
        controllers: &[String],
        property_path: &str,
        is_alias: bool,
    ) -> Option<GotoDefinitionResponse> {
        // 1. `{ctrl}.$scope.{prop}` を各 controller で試す
        for controller_name in controllers {
            let symbol_name = format!("{}.$scope.{}", controller_name, property_path);
            let definitions = self.index.definitions.get_definitions(&symbol_name);
            if !definitions.is_empty() {
                return Some(GotoDefinitionResponse::Array(
                    definitions
                        .into_iter()
                        .map(|def| Location {
                            uri: def.uri.clone(),
                            range: def.definition_span.to_lsp_range(),
                        })
                        .collect(),
                ));
            }
        }

        // 2. controller as 構文の場合は `{ctrl}.{prop}` (this.method) も試す
        if is_alias {
            for controller_name in controllers {
                let symbol_name = format!("{}.{}", controller_name, property_path);
                let definitions = self.index.definitions.get_definitions(&symbol_name);
                if !definitions.is_empty() {
                    return Some(GotoDefinitionResponse::Array(
                        definitions
                            .into_iter()
                            .map(|def| Location {
                                uri: def.uri.clone(),
                                range: def.definition_span.to_lsp_range(),
                            })
                            .collect(),
                    ));
                }
            }
        }

        // 3. $rootScope からのグローバル参照
        let root_scope_defs = self
            .index
            .definitions
            .find_root_scope_definitions_by_property(property_path);
        if !root_scope_defs.is_empty() {
            return Some(GotoDefinitionResponse::Array(
                root_scope_defs
                    .into_iter()
                    .map(|def| Location {
                        uri: def.uri.clone(),
                        range: def.definition_span.to_lsp_range(),
                    })
                    .collect(),
            ));
        }

        // 4. ng-model 経由の暗黙的 scope 定義 (controller 側で `$scope.X = ...` を
        //    書かなくても <input ng-model="X"> があれば AngularJS が自動生成するため)
        for controller_name in controllers {
            if let Some(target) =
                self.index
                    .find_ng_model_implicit_def_target(uri, controller_name, property_path)
            {
                debug!(
                    "goto_definition_from_html: '{}' resolved via ng-model implicit def at {}:{}",
                    property_path, target.start_line, target.start_col
                );
                return Some(GotoDefinitionResponse::Scalar(Location {
                    uri: target.uri.clone(),
                    range: target.span().to_lsp_range(),
                }));
            }
        }

        None
    }
}

fn scalar(uri: &Url, range: Range) -> GotoDefinitionResponse {
    GotoDefinitionResponse::Scalar(Location {
        uri: uri.clone(),
        range,
    })
}
