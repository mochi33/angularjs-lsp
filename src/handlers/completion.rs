use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::index::{SymbolIndex, SymbolKind};

pub struct CompletionHandler {
    index: Arc<SymbolIndex>,
}

impl CompletionHandler {
    pub fn new(index: Arc<SymbolIndex>) -> Self {
        Self { index }
    }

    /// サービスプレフィックスに基づいて補完候補を返す
    /// service_prefix: "ServiceName" の場合、"ServiceName.xxx" のメソッドのみ返す
    pub fn complete_with_context(&self, service_prefix: Option<&str>) -> Option<CompletionResponse> {
        let definitions = self.index.get_all_definitions();

        let items: Vec<CompletionItem> = if let Some(prefix) = service_prefix {
            // サービス名が指定された場合、そのサービスのメソッドのみを返す
            let method_prefix = format!("{}.", prefix);
            definitions
                .into_iter()
                .filter(|s| s.name.starts_with(&method_prefix))
                .map(|symbol| {
                    // "ServiceName.methodName" から "methodName" だけを抽出
                    let method_name = symbol.name.strip_prefix(&method_prefix)
                        .unwrap_or(&symbol.name)
                        .to_string();

                    CompletionItem {
                        label: method_name,
                        kind: Some(CompletionItemKind::METHOD),
                        detail: Some(format!("{} ({})", prefix, symbol.kind.as_str())),
                        documentation: symbol.docs.map(|docs| {
                            Documentation::MarkupContent(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value: docs,
                            })
                        }),
                        ..Default::default()
                    }
                })
                .collect()
        } else {
            // 通常の補完: 全シンボルを返す（メソッドは除外）
            definitions
                .into_iter()
                .filter(|s| s.kind != SymbolKind::Method)
                .map(|symbol| {
                    let kind = self.symbol_kind_to_completion_kind(symbol.kind);
                    let detail = symbol.kind.as_str().to_string();

                    CompletionItem {
                        label: symbol.name.clone(),
                        kind: Some(kind),
                        detail: Some(detail),
                        documentation: symbol.docs.map(|docs| {
                            Documentation::MarkupContent(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value: docs,
                            })
                        }),
                        ..Default::default()
                    }
                })
                .collect()
        };

        Some(CompletionResponse::Array(items))
    }

    fn symbol_kind_to_completion_kind(&self, kind: SymbolKind) -> CompletionItemKind {
        match kind {
            SymbolKind::Module => CompletionItemKind::MODULE,
            SymbolKind::Controller => CompletionItemKind::CLASS,
            SymbolKind::Service => CompletionItemKind::CLASS,
            SymbolKind::Factory => CompletionItemKind::CLASS,
            SymbolKind::Directive => CompletionItemKind::CLASS,
            SymbolKind::Provider => CompletionItemKind::CLASS,
            SymbolKind::Filter => CompletionItemKind::FUNCTION,
            SymbolKind::Constant => CompletionItemKind::CONSTANT,
            SymbolKind::Value => CompletionItemKind::VALUE,
            SymbolKind::Method => CompletionItemKind::METHOD,
        }
    }
}
