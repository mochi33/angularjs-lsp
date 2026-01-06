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

    pub fn hover(&self, params: HoverParams) -> Option<Hover> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let symbol_name =
            self.index
                .find_symbol_at_position(&uri, position.line, position.character)?;

        let definitions = self.index.get_definitions(&symbol_name);

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

        let reference_count = self.index.get_references(&symbol_name).len();

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
