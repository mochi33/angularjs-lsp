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

        let command = if let Some(def) = definitions.first() {
            // 定義が見つかった場合、クリックでジャンプ可能
            Command {
                title: format!("{}: {}", source, controller_name),
                command: "editor.action.goToLocations".to_string(),
                arguments: Some(vec![
                    serde_json::to_value(def.uri.to_string()).unwrap(),
                    serde_json::to_value(Position {
                        line: def.start_line,
                        character: def.start_col,
                    }).unwrap(),
                    serde_json::to_value(Vec::<Location>::new()).unwrap(),
                    serde_json::to_value("goto").unwrap(),
                    serde_json::to_value("No definition found").unwrap(),
                ]),
            }
        } else {
            // 定義が見つからない場合
            Command {
                title: format!("{}: {} (not found)", source, controller_name),
                command: "".to_string(),
                arguments: None,
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

        // 複数のテンプレートがある場合は最初のものにジャンプ
        // TODO: 複数選択のUIを検討
        let command = Command {
            title,
            command: "".to_string(),  // クリックしても何もしない（表示のみ）
            arguments: None,
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
