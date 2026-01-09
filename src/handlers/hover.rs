use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::index::SymbolIndex;

pub struct HoverHandler {
    index: Arc<SymbolIndex>,
}

impl HoverHandler {
    pub fn new(index: Arc<SymbolIndex>) -> Self {
        Self { index }
    }

    /// ファイルがHTMLかどうか判定
    fn is_html_file(uri: &Url) -> bool {
        let path = uri.path().to_lowercase();
        path.ends_with(".html") || path.ends_with(".htm")
    }

    pub fn hover(&self, params: HoverParams) -> Option<Hover> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        // HTMLファイルの場合は専用の処理
        if Self::is_html_file(&uri) {
            return self.hover_from_html(&uri, position);
        }

        let symbol_name =
            self.index
                .find_symbol_at_position(&uri, position.line, position.character)?;

        self.build_hover_for_symbol(&symbol_name)
    }

    /// HTMLファイルからのホバー
    fn hover_from_html(&self, uri: &Url, position: Position) -> Option<Hover> {
        // 1. 位置からHTMLスコープ参照を取得
        let html_ref = self.index.find_html_scope_reference_at(
            uri,
            position.line,
            position.character,
        )?;

        // 2. コントローラー名を解決（複数の可能性あり）
        let controllers = self.index.resolve_controllers_for_html(uri, position.line);

        // 3. 各コントローラーを順番に試して、定義が見つかったものを返す
        for controller_name in controllers {
            let symbol_name = format!(
                "{}.$scope.{}",
                controller_name,
                html_ref.property_path
            );

            if let Some(hover) = self.build_hover_for_symbol(&symbol_name) {
                return Some(hover);
            }
        }

        None
    }

    /// シンボル名からホバー情報を構築
    fn build_hover_for_symbol(&self, symbol_name: &str) -> Option<Hover> {
        let definitions = self.index.get_definitions(symbol_name);

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

        content.push_str(&format!("Defined in: `{}:{}`\n", file_name, def.start_line + 1));

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
}
