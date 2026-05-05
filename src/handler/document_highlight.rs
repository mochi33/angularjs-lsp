//! `textDocument/documentHighlight` の実装。
//!
//! カーソル位置のシンボルが「同一ファイル内」で使われている全箇所を
//! [`DocumentHighlight`] のリストとして返す。`references` (workspace 全体) と
//! ロジックは似ているが、URI フィルタを掛けて 同 URI のみに絞る。
//!
//! `kind` フィールドの選択ルール:
//! - 定義位置 (`Symbol`) → [`DocumentHighlightKind::WRITE`]
//! - 参照位置 (`SymbolReference` / HTML 参照) → [`DocumentHighlightKind::READ`]
//!
//! 継承された var (親テンプレート由来) も「ユーザーから見える定義」が
//! このファイル上にないため `READ` 扱い。`TEXT` は現状使用しない。
use std::sync::Arc;

use tower_lsp::lsp_types::*;
use tracing::debug;

use crate::index::{HtmlResolution, Index};
use crate::model::{HtmlFormBinding, HtmlLocalVariable, HtmlUiSrefReference, SymbolKind};
use crate::util::is_html_file;

pub struct DocumentHighlightHandler {
    index: Arc<Index>,
}

impl DocumentHighlightHandler {
    pub fn new(index: Arc<Index>) -> Self {
        Self { index }
    }

    pub fn document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> Option<Vec<DocumentHighlight>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        if is_html_file(&uri) {
            return self.highlight_from_html(&uri, position);
        }

        // JS ファイル: シンボル名を取り出し、同 URI のみフィルタして返す
        let symbol_name = self.index.definitions.find_symbol_at_position(
            &uri,
            position.line,
            position.character,
        )?;

        self.collect_symbol_highlights_in_uri(&uri, &symbol_name)
    }

    /// HTML ファイルのカーソル位置を解決し、同 URI 内の参照のみハイライト
    fn highlight_from_html(&self, uri: &Url, position: Position) -> Option<Vec<DocumentHighlight>> {
        match self.index.resolve_html_position(uri, position, None)? {
            HtmlResolution::UiSref(r) => self.highlight_for_ui_sref(uri, &r),
            HtmlResolution::Directive(r) => {
                self.highlight_for_directive(uri, &r.directive_name)
            }
            HtmlResolution::LocalVarDef(v) | HtmlResolution::LocalVarRef(v) => {
                self.highlight_for_local_variable(uri, &v)
            }
            HtmlResolution::InheritedLocalVar(v) => {
                // 親テンプレートで定義されたローカル変数: この URI 上の参照のみ拾う
                self.highlight_for_inherited_local_variable(uri, &v)
            }
            HtmlResolution::FormBindingDef(f) | HtmlResolution::InheritedFormBinding(f) => {
                self.highlight_for_form_binding(uri, &f)
            }
            HtmlResolution::Scope {
                controllers,
                property_path,
                is_alias,
            } => self.highlight_for_scope(uri, &controllers, &property_path, is_alias),
        }
    }

    /// `ui-sref="state"` の state 名を同 URI でハイライト
    fn highlight_for_ui_sref(
        &self,
        uri: &Url,
        ui_sref: &HtmlUiSrefReference,
    ) -> Option<Vec<DocumentHighlight>> {
        let mut highlights = Vec::new();

        // 同 URI 内の同名 ui-sref をすべて拾う
        for reference in self.index.definitions.get_references(&ui_sref.state_name) {
            if &reference.uri == uri {
                highlights.push(DocumentHighlight {
                    range: reference.span.to_lsp_range(),
                    kind: Some(DocumentHighlightKind::READ),
                });
            }
        }

        // state 定義が同 URI にある場合 (HTML から定義することはまずないが対称性のため)
        for def in self.index.definitions.get_definitions(&ui_sref.state_name) {
            if &def.uri == uri && def.kind == SymbolKind::UiRouterState {
                highlights.push(DocumentHighlight {
                    range: def.definition_span.to_lsp_range(),
                    kind: Some(DocumentHighlightKind::WRITE),
                });
            }
        }

        finalize(highlights)
    }

    /// カスタムディレクティブ / コンポーネント参照を同 URI でハイライト
    fn highlight_for_directive(
        &self,
        uri: &Url,
        directive_name: &str,
    ) -> Option<Vec<DocumentHighlight>> {
        let mut highlights = Vec::new();

        // 定義は通常 JS にあるので同 URI には載らないが念のため対称的に処理
        for def in self.index.definitions.get_definitions(directive_name) {
            if &def.uri == uri
                && (def.kind == SymbolKind::Directive || def.kind == SymbolKind::Component)
            {
                highlights.push(DocumentHighlight {
                    range: def.definition_span.to_lsp_range(),
                    kind: Some(DocumentHighlightKind::WRITE),
                });
            }
        }

        for reference in self
            .index
            .html
            .get_html_directive_references(directive_name)
        {
            if &reference.uri == uri {
                highlights.push(DocumentHighlight {
                    range: reference.span().to_lsp_range(),
                    kind: Some(DocumentHighlightKind::READ),
                });
            }
        }

        // JS 側のテキスト参照は JS パス (`collect_symbol_highlights_in_uri`)
        // で `get_all_references` から拾われる。`highlight_from_html` 経路では
        // 同 URI フィルタで JS ref はどのみち落ちるため、ここでは省略している。
        finalize(highlights)
    }

    /// ローカル変数 (ng-repeat / ng-init / let-) の定義 + scope 内参照をハイライト
    fn highlight_for_local_variable(
        &self,
        uri: &Url,
        var_def: &HtmlLocalVariable,
    ) -> Option<Vec<DocumentHighlight>> {
        let mut highlights = Vec::new();

        // 定義位置 (定義 URI = 現在 URI のはず)
        if &var_def.uri == uri {
            highlights.push(DocumentHighlight {
                range: var_def.name_span().to_lsp_range(),
                kind: Some(DocumentHighlightKind::WRITE),
            });
        }

        // scope 範囲内の参照
        let refs = self.index.html.get_local_variable_references(
            &var_def.uri,
            &var_def.name,
            var_def.scope_start_line,
            var_def.scope_end_line,
        );
        for reference in refs {
            if &reference.uri == uri {
                highlights.push(DocumentHighlight {
                    range: reference.span().to_lsp_range(),
                    kind: Some(DocumentHighlightKind::READ),
                });
            }
        }

        finalize(highlights)
    }

    /// 継承された (親テンプレート由来の) ローカル変数: 現在 URI 内の参照のみ
    fn highlight_for_inherited_local_variable(
        &self,
        uri: &Url,
        var_def: &HtmlLocalVariable,
    ) -> Option<Vec<DocumentHighlight>> {
        let mut highlights = Vec::new();

        // 継承元の定義は別 URI なのでハイライトしない。
        // この URI 上には「参照」しか見えないので READ で統一する。
        let inherited_refs = self
            .index
            .templates
            .get_inherited_local_variable_references(
                &var_def.uri,
                &var_def.name,
                self.index.html.html_local_variable_references_raw(),
            );

        for reference in inherited_refs {
            if &reference.uri == uri {
                highlights.push(DocumentHighlight {
                    range: reference.span().to_lsp_range(),
                    kind: Some(DocumentHighlightKind::READ),
                });
            }
        }

        finalize(highlights)
    }

    /// `<form name="x">` のフォームバインディングを同 URI でハイライト
    fn highlight_for_form_binding(
        &self,
        uri: &Url,
        form_binding: &HtmlFormBinding,
    ) -> Option<Vec<DocumentHighlight>> {
        let mut highlights = Vec::new();

        // 定義位置 (form name 属性値)
        if &form_binding.uri == uri {
            highlights.push(DocumentHighlight {
                range: form_binding.name_span().to_lsp_range(),
                kind: Some(DocumentHighlightKind::WRITE),
            });
        }

        let controllers = self
            .index
            .resolve_controllers_for_html(&form_binding.uri, form_binding.scope_start_line);
        for controller_name in controllers {
            let symbol_name = format!("{}.$scope.{}", controller_name, form_binding.name);

            for reference in self.index.get_html_references_for_symbol(&symbol_name) {
                if &reference.uri == uri
                    && reference.span.start_line >= form_binding.scope_start_line
                    && reference.span.start_line <= form_binding.scope_end_line
                {
                    highlights.push(DocumentHighlight {
                        range: reference.span.to_lsp_range(),
                        kind: Some(DocumentHighlightKind::READ),
                    });
                }
            }

            // JS 側の `$scope.formName` 参照も拾う (references.rs と同構造)。
            // 異 URI は同 URI フィルタで落ちるが、HTML/JS 両方から同じ form を
            // 触るケースで対称性を保つ。
            for reference in self.index.definitions.get_references(&symbol_name) {
                if &reference.uri == uri {
                    highlights.push(DocumentHighlight {
                        range: reference.span.to_lsp_range(),
                        kind: Some(DocumentHighlightKind::READ),
                    });
                }
            }
        }

        finalize(highlights)
    }

    /// `$scope` プロパティ参照: `{ctrl}.$scope.{prop}` → (alias なら) `{ctrl}.{prop}`
    /// → `$rootScope.{prop}` のチェインを試し、最初にヒットしたシンボル名で
    /// 同 URI 内をハイライトする。
    fn highlight_for_scope(
        &self,
        uri: &Url,
        controllers: &[String],
        property_path: &str,
        is_alias: bool,
    ) -> Option<Vec<DocumentHighlight>> {
        for controller_name in controllers {
            let symbol_name = format!("{}.$scope.{}", controller_name, property_path);
            if self.index.definitions.has_definition(&symbol_name) {
                if let Some(highlights) =
                    self.collect_symbol_highlights_in_uri(uri, &symbol_name)
                {
                    return Some(highlights);
                }
            }
        }

        if is_alias {
            for controller_name in controllers {
                let symbol_name = format!("{}.{}", controller_name, property_path);
                if self.index.definitions.has_definition(&symbol_name) {
                    if let Some(highlights) =
                        self.collect_symbol_highlights_in_uri(uri, &symbol_name)
                    {
                        return Some(highlights);
                    }
                }
            }
        }

        if let Some(root_scope_symbol) = self
            .index
            .definitions
            .find_root_scope_symbol_name_by_property(property_path)
        {
            debug!(
                "highlight_for_scope: found $rootScope symbol '{}'",
                root_scope_symbol
            );
            return self.collect_symbol_highlights_in_uri(uri, &root_scope_symbol);
        }

        None
    }

    /// シンボル名から JS 定義 + JS 参照 + HTML 参照を集め、同 URI のものだけ返す
    fn collect_symbol_highlights_in_uri(
        &self,
        uri: &Url,
        symbol_name: &str,
    ) -> Option<Vec<DocumentHighlight>> {
        let mut highlights = Vec::new();

        for def in self.index.definitions.get_definitions(symbol_name) {
            if &def.uri == uri {
                highlights.push(DocumentHighlight {
                    range: def.definition_span.to_lsp_range(),
                    kind: Some(DocumentHighlightKind::WRITE),
                });
            }
        }

        for reference in self.index.get_all_references(symbol_name) {
            if &reference.uri == uri {
                highlights.push(DocumentHighlight {
                    range: reference.span.to_lsp_range(),
                    kind: Some(DocumentHighlightKind::READ),
                });
            }
        }

        finalize(highlights)
    }
}

fn finalize(mut highlights: Vec<DocumentHighlight>) -> Option<Vec<DocumentHighlight>> {
    // 同じ range が WRITE/READ で重複し得るので、range で de-dup する。
    // WRITE を優先したいので先にソート (Some(WRITE) < Some(READ) < Some(TEXT) は
    // LSP enum 値順で 1 < 2 < 3、つまり WRITE が小さい)。
    highlights.sort_by(|a, b| {
        let key_a = (a.range.start, a.range.end, a.kind.map(kind_priority));
        let key_b = (b.range.start, b.range.end, b.kind.map(kind_priority));
        key_a.cmp(&key_b)
    });
    highlights.dedup_by(|a, b| a.range == b.range);

    if highlights.is_empty() {
        None
    } else {
        Some(highlights)
    }
}

/// 同じ range で重複したとき WRITE > READ > TEXT > None を残すための優先度。
/// 値が小さいほど優先される (sort 後 `dedup_by` は最初の要素を残す挙動)。
fn kind_priority(kind: DocumentHighlightKind) -> u8 {
    match kind {
        DocumentHighlightKind::WRITE => 0,
        DocumentHighlightKind::READ => 1,
        DocumentHighlightKind::TEXT => 2,
        _ => 3,
    }
}
