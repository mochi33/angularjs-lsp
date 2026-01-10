use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::index::{SymbolIndex, SymbolKind};

pub struct CodeLensHandler {
    index: Arc<SymbolIndex>,
}

impl CodeLensHandler {
    pub fn new(index: Arc<SymbolIndex>) -> Self {
        Self { index }
    }

    /// ファイルがHTMLかどうか判定
    fn is_html_file(uri: &Url) -> bool {
        let path = uri.path().to_lowercase();
        path.ends_with(".html") || path.ends_with(".htm")
    }

    /// ファイルがJSかどうか判定
    fn is_js_file(uri: &Url) -> bool {
        uri.path().ends_with(".js")
    }

    pub fn code_lens(&self, uri: &Url) -> Option<Vec<CodeLens>> {
        if Self::is_html_file(uri) {
            self.code_lens_for_html(uri)
        } else if Self::is_js_file(uri) {
            self.code_lens_for_js(uri)
        } else {
            None
        }
    }

    /// HTMLファイル用のCodeLens
    fn code_lens_for_html(&self, uri: &Url) -> Option<Vec<CodeLens>> {
        let mut lenses = Vec::new();

        // 1. テンプレートバインディング経由のコントローラー
        if let Some(controller) = self.index.get_controller_for_template(uri) {
            lenses.push(self.create_controller_lens(&controller, 0, "Controller"));
        }

        // 2. ng-include継承コントローラー
        let inherited = self.index.get_inherited_controllers_for_template(uri);
        for controller in inherited {
            lenses.push(self.create_controller_lens(&controller, 0, "Inherited"));
        }

        // 3. HTML内のng-controllerスコープ
        let html_scopes = self.index.get_all_html_controller_scopes(uri);
        for scope in html_scopes {
            lenses.push(self.create_controller_lens(
                &scope.controller_name,
                scope.start_line,
                "ng-controller",
            ));
        }

        if lenses.is_empty() {
            None
        } else {
            Some(lenses)
        }
    }

    /// JSファイル用のCodeLens（コントローラーにバインドされたHTMLを表示）
    fn code_lens_for_js(&self, uri: &Url) -> Option<Vec<CodeLens>> {
        let mut lenses = Vec::new();

        // このファイル内のコントローラー定義を取得
        let symbols = self.index.get_document_symbols(uri);
        for symbol in symbols {
            if symbol.kind == SymbolKind::Controller {
                let templates = self.index.get_templates_for_controller(&symbol.name);
                if !templates.is_empty() {
                    lenses.push(self.create_template_lens(&symbol.name, symbol.name_start_line, &templates));
                }
            }
        }

        if lenses.is_empty() {
            None
        } else {
            Some(lenses)
        }
    }

    fn create_controller_lens(&self, controller_name: &str, line: u32, source: &str) -> CodeLens {
        // コントローラー定義を検索
        let definitions = self.index.get_definitions(controller_name);

        let command = if definitions.is_empty() {
            // 定義が見つからない場合
            Command {
                title: format!("{}: {} (not found)", source, controller_name),
                command: "".to_string(),
                arguments: None,
            }
        } else {
            // Locationの配列を作成
            let locations: Vec<serde_json::Value> = definitions
                .iter()
                .map(|def| {
                    serde_json::json!({
                        "uri": def.uri.to_string(),
                        "range": {
                            "start": { "line": def.start_line, "character": def.start_col },
                            "end": { "line": def.start_line, "character": def.start_col }
                        }
                    })
                })
                .collect();

            Command {
                title: format!("{}: {}", source, controller_name),
                command: "angularjs.openLocation".to_string(),
                arguments: Some(vec![serde_json::json!(locations)]),
            }
        };

        CodeLens {
            range: Range {
                start: Position { line, character: 0 },
                end: Position { line, character: 0 },
            },
            command: Some(command),
            data: None,
        }
    }

    /// JSコントローラー用のCodeLens（バインドされたHTMLテンプレートを表示）
    fn create_template_lens(&self, _controller_name: &str, line: u32, templates: &[String]) -> CodeLens {
        // テンプレートパスからファイル名のみを抽出して表示
        let template_names: Vec<String> = templates
            .iter()
            .map(|t| t.rsplit('/').next().unwrap_or(t).to_string())
            .collect();

        let title = if template_names.len() == 1 {
            format!("Template: {}", template_names[0])
        } else {
            format!("Templates: {}", template_names.join(", "))
        };

        // テンプレートURIを取得してLocationの配列を作成
        let locations: Vec<serde_json::Value> = templates
            .iter()
            .filter_map(|template_path| {
                self.index.resolve_template_uri(template_path).map(|uri| {
                    serde_json::json!({
                        "uri": uri.to_string(),
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end": { "line": 0, "character": 0 }
                        }
                    })
                })
            })
            .collect();

        let command = if locations.is_empty() {
            Command {
                title,
                command: "".to_string(),
                arguments: None,
            }
        } else {
            Command {
                title,
                command: "angularjs.openLocation".to_string(),
                arguments: Some(vec![serde_json::json!(locations)]),
            }
        };

        CodeLens {
            range: Range {
                start: Position { line, character: 0 },
                end: Position { line, character: 0 },
            },
            command: Some(command),
            data: None,
        }
    }
}
