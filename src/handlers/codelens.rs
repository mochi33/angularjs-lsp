use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::index::{BindingSource, SymbolIndex, SymbolKind};

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

        // 1. このファイルをng-includeしている親ファイル（ファイル先頭に表示）
        let parents = self.index.get_parent_templates_for_child(uri);
        if !parents.is_empty() {
            lenses.push(self.create_included_by_lens(&parents));
        }

        // 2. テンプレートバインディング経由の呼び出し元（JSファイル）
        if let Some((controller_name, source, js_uri, js_line)) = self.index.get_template_binding_source(uri) {
            lenses.push(self.create_bound_from_lens(&controller_name, &source, &js_uri, js_line));
        }

        // 3. ng-include継承コントローラー
        let inherited = self.index.get_inherited_controllers_for_template(uri);
        for controller in inherited {
            lenses.push(self.create_controller_lens(&controller, 0, "Inherited"));
        }

        // 4. HTML内のng-controllerスコープ
        let html_scopes = self.index.get_all_html_controller_scopes(uri);
        for scope in html_scopes {
            lenses.push(self.create_controller_lens(
                &scope.controller_name,
                scope.start_line,
                "ng-controller",
            ));
        }

        // 5. ng-includeの呼び出し先ファイル（各ng-includeの行に表示）
        let ng_includes = self.index.get_ng_includes_in_file(uri);
        for (line, template_path, resolved_uri) in ng_includes {
            lenses.push(self.create_ng_include_lens(line, &template_path, resolved_uri));
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
                title: format!("{} (not resolved)", title),
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

    /// 呼び出し元（親ファイル）へのCodeLens（ファイル先頭に表示）
    fn create_included_by_lens(&self, parents: &[(Url, u32)]) -> CodeLens {
        // ファイル名のみを抽出
        let parent_names: Vec<String> = parents
            .iter()
            .map(|(uri, _)| {
                uri.path()
                    .rsplit('/')
                    .next()
                    .unwrap_or("unknown")
                    .to_string()
            })
            .collect();

        let title = if parent_names.len() == 1 {
            format!("Included by: {}", parent_names[0])
        } else {
            format!("Included by: {}", parent_names.join(", "))
        };

        // 親ファイルのng-include位置へのLocationリストを作成
        let locations: Vec<serde_json::Value> = parents
            .iter()
            .map(|(uri, line)| {
                serde_json::json!({
                    "uri": uri.to_string(),
                    "range": {
                        "start": { "line": line, "character": 0 },
                        "end": { "line": line, "character": 0 }
                    }
                })
            })
            .collect();

        let command = Command {
            title,
            command: "angularjs.openLocation".to_string(),
            arguments: Some(vec![serde_json::json!(locations)]),
        };

        CodeLens {
            range: Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 0, character: 0 },
            },
            command: Some(command),
            data: None,
        }
    }

    /// テンプレートバインディングの呼び出し元（JSファイル）へのCodeLens
    fn create_bound_from_lens(&self, controller_name: &str, source: &BindingSource, js_uri: &Url, js_line: u32) -> CodeLens {
        let js_filename = js_uri.path().rsplit('/').next().unwrap_or("unknown");

        let source_label = match source {
            BindingSource::RouteProvider => "$routeProvider",
            BindingSource::UibModal => "$uibModal",
            BindingSource::NgController => "ng-controller",
        };

        let title = format!("Bound from: {} ({} in {})", controller_name, source_label, js_filename);

        let locations = vec![serde_json::json!({
            "uri": js_uri.to_string(),
            "range": {
                "start": { "line": js_line, "character": 0 },
                "end": { "line": js_line, "character": 0 }
            }
        })];

        let command = Command {
            title,
            command: "angularjs.openLocation".to_string(),
            arguments: Some(vec![serde_json::json!(locations)]),
        };

        CodeLens {
            range: Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 0, character: 0 },
            },
            command: Some(command),
            data: None,
        }
    }

    /// ng-includeの呼び出し先ファイルへのCodeLens
    fn create_ng_include_lens(&self, line: u32, template_path: &str, resolved_uri: Option<Url>) -> CodeLens {
        // ファイル名のみを抽出
        let filename = template_path.rsplit('/').next().unwrap_or(template_path);

        let command = if let Some(uri) = resolved_uri {
            let locations = vec![serde_json::json!({
                "uri": uri.to_string(),
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end": { "line": 0, "character": 0 }
                }
            })];

            Command {
                title: format!("ng-include: {}", filename),
                command: "angularjs.openLocation".to_string(),
                arguments: Some(vec![serde_json::json!(locations)]),
            }
        } else {
            Command {
                title: format!("ng-include: {} (not found)", filename),
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
}
