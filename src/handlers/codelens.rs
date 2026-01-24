use std::collections::HashSet;
use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::index::{BindingSource, ComponentTemplateUrl, SymbolIndex, SymbolKind, TemplateBinding};

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
        // ng-view-virtual-parentは内部用なので除外
        let parents: Vec<_> = self.index.get_parent_templates_for_child(uri)
            .into_iter()
            .filter(|(url, _)| !url.path().contains("ng-view-virtual-parent"))
            .collect();
        if !parents.is_empty() {
            lenses.push(self.create_included_by_lens(&parents));
        }

        // 2. テンプレートバインディング経由の呼び出し元（JSファイル）
        // 同じコントローラー・同じファイルの重複を排除
        let bindings = self.index.get_all_template_binding_sources(uri);
        let mut seen_files: HashSet<String> = HashSet::new();
        let mut seen_controllers: HashSet<String> = HashSet::new();
        for (controller_name, source, js_uri, js_line) in bindings {
            // バインディング箇所へのジャンプ（同じファイルは1回だけ）
            let file_key = js_uri.to_string();
            if !seen_files.contains(&file_key) {
                seen_files.insert(file_key);
                lenses.push(self.create_bound_from_lens(&source, &js_uri, js_line));
            }
            // コントローラー定義へのジャンプ（同じコントローラーは1回だけ）
            if !seen_controllers.contains(&controller_name) {
                seen_controllers.insert(controller_name.clone());
                lenses.push(self.create_controller_lens(&controller_name, 0, "Binded"));
            }
        }

        // 3. コンポーネントテンプレートのコントローラー（ファイル先頭に表示）
        if let Some(binding) = self.index.get_component_binding_for_template(uri) {
            lenses.push(self.create_component_controller_lens(&binding));
        }

        // 4. ng-include継承コントローラー
        let inherited = self.index.get_inherited_controllers_for_template(uri);
        for controller in inherited {
            lenses.push(self.create_controller_lens(&controller, 0, "Inherited"));
        }

        // 5. HTML内のng-controllerスコープ
        let html_scopes = self.index.get_all_html_controller_scopes(uri);
        for scope in html_scopes {
            lenses.push(self.create_controller_lens(
                &scope.controller_name,
                scope.start_line,
                "ng-controller",
            ));
        }

        // 6. ng-includeの呼び出し先ファイル（各ng-includeの行に表示）
        let ng_includes = self.index.get_ng_includes_in_file(uri);
        for (line, template_path, resolved_uri) in ng_includes {
            lenses.push(self.create_ng_include_lens(line, &template_path, resolved_uri));
        }

        // 7. <script>タグ内のコントローラー定義（JSファイルと同様の処理）
        let symbols = self.index.get_document_symbols(uri);
        for symbol in symbols {
            if symbol.kind == SymbolKind::Controller {
                let templates = self.index.get_templates_for_controller(&symbol.name);
                if !templates.is_empty() {
                    lenses.push(self.create_template_lens(&symbol.name, symbol.name_start_line, &templates));
                }
            }
        }

        // 8. <script>タグ内のテンプレートバインディング定義
        let bindings = self.index.get_template_bindings_for_js_file(uri);
        for binding in bindings {
            lenses.push(self.create_binding_lens(&binding));
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

        // このファイル内のテンプレートバインディング定義を取得
        let bindings = self.index.get_template_bindings_for_js_file(uri);
        for binding in bindings {
            lenses.push(self.create_binding_lens(&binding));
        }

        // このファイル内のコンポーネント templateUrl を取得
        let template_urls = self.index.get_component_template_urls(uri);
        for template_url in template_urls {
            lenses.push(self.create_component_template_url_lens(&template_url));
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
    fn create_bound_from_lens(&self, source: &BindingSource, js_uri: &Url, js_line: u32) -> CodeLens {
        let js_filename = js_uri.path().rsplit('/').next().unwrap_or("unknown");

        let source_label = match source {
            BindingSource::RouteProvider => "$routeProvider",
            BindingSource::UibModal => "$uibModal",
            BindingSource::NgController => "ng-controller",
        };

        let title = format!("Bound from: {} in {}", source_label, js_filename);

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

    /// テンプレートバインディング定義箇所のCodeLens（templateファイルとControllerファイルにジャンプ）
    fn create_binding_lens(&self, binding: &TemplateBinding) -> CodeLens {
        let template_filename = binding.template_path.rsplit('/').next().unwrap_or(&binding.template_path);

        let source_label = match binding.source {
            BindingSource::RouteProvider => "$routeProvider",
            BindingSource::UibModal => "$uibModal",
            BindingSource::NgController => "ng-controller",
        };

        let title = format!("{}: {} -> {}", source_label, template_filename, binding.controller_name);

        let mut locations: Vec<serde_json::Value> = Vec::new();

        // templateファイルへのLocation
        if let Some(template_uri) = self.index.resolve_template_uri(&binding.template_path) {
            locations.push(serde_json::json!({
                "uri": template_uri.to_string(),
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end": { "line": 0, "character": 0 }
                }
            }));
        }

        // Controllerファイルへのocation
        let definitions = self.index.get_definitions(&binding.controller_name);
        for def in definitions {
            locations.push(serde_json::json!({
                "uri": def.uri.to_string(),
                "range": {
                    "start": { "line": def.start_line, "character": def.start_col },
                    "end": { "line": def.start_line, "character": def.start_col }
                }
            }));
        }

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
                start: Position { line: binding.binding_line, character: 0 },
                end: Position { line: binding.binding_line, character: 0 },
            },
            command: Some(command),
            data: None,
        }
    }

    /// コンポーネントテンプレートのコントローラー表示用CodeLens（HTMLファイル先頭に表示）
    fn create_component_controller_lens(&self, binding: &ComponentTemplateUrl) -> CodeLens {
        let controller_display = if let Some(ref controller_name) = binding.controller_name {
            controller_name.clone()
        } else {
            "(inline)".to_string()
        };

        let title = format!(
            "Component: {} (as {})",
            controller_display, binding.controller_as
        );

        // コントローラー定義を検索
        let command = if let Some(ref controller_name) = binding.controller_name {
            let definitions = self.index.get_definitions(controller_name);
            if definitions.is_empty() {
                Command {
                    title: format!("{} (not found)", title),
                    command: "".to_string(),
                    arguments: None,
                }
            } else {
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
                    title,
                    command: "angularjs.openLocation".to_string(),
                    arguments: Some(vec![serde_json::json!(locations)]),
                }
            }
        } else {
            // インラインコントローラーの場合、コンポーネント定義にジャンプ
            let locations = vec![serde_json::json!({
                "uri": binding.uri.to_string(),
                "range": {
                    "start": { "line": binding.line, "character": binding.col },
                    "end": { "line": binding.line, "character": binding.col }
                }
            })];

            Command {
                title,
                command: "angularjs.openLocation".to_string(),
                arguments: Some(vec![serde_json::json!(locations)]),
            }
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

    /// コンポーネントのtemplateUrl用のCodeLens
    fn create_component_template_url_lens(&self, template_url: &ComponentTemplateUrl) -> CodeLens {
        let template_filename = template_url
            .template_path
            .rsplit('/')
            .next()
            .unwrap_or(&template_url.template_path);

        let title = format!("Open template: {}", template_filename);

        // テンプレートURIを解決
        let resolved_uri = self.index.resolve_template_uri(&template_url.template_path);

        let command = if let Some(uri) = resolved_uri {
            let locations = vec![serde_json::json!({
                "uri": uri.to_string(),
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end": { "line": 0, "character": 0 }
                }
            })];

            Command {
                title,
                command: "angularjs.openLocation".to_string(),
                arguments: Some(vec![serde_json::json!(locations)]),
            }
        } else {
            Command {
                title: format!("{} (not found)", title),
                command: "".to_string(),
                arguments: None,
            }
        };

        CodeLens {
            range: Range {
                start: Position {
                    line: template_url.line,
                    character: 0,
                },
                end: Position {
                    line: template_url.line,
                    character: 0,
                },
            },
            command: Some(command),
            data: None,
        }
    }
}
