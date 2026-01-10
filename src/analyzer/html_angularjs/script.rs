//! <script>タグ内のJavaScript解析

use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::JsParser;
use crate::index::{BindingSource, TemplateBinding};

use super::HtmlAngularJsAnalyzer;

impl HtmlAngularJsAnalyzer {
    /// <script>タグ内のJavaScriptを解析
    pub(super) fn analyze_script_tags(&self, node: Node, source: &str, uri: &Url) {
        if node.kind() == "script_element" {
            // <script>タグの内容を取得
            if let Some(raw_text) = self.find_child_by_kind(node, "raw_text") {
                let js_source = self.node_text(raw_text, source);
                // scriptタグの開始行をオフセットとして使用
                let line_offset = raw_text.start_position().row as u32;

                // AngularJsAnalyzerで完全な解析を実行
                self.js_analyzer.analyze_embedded_script(uri, &js_source, line_offset);

                // テンプレートバインディングも抽出
                self.analyze_embedded_js(&js_source);
            }
        }

        // 子ノードを再帰的に処理
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.analyze_script_tags(child, source, uri);
        }
    }

    /// 埋め込みJavaScriptからテンプレートバインディングを抽出
    fn analyze_embedded_js(&self, js_source: &str) {
        let mut parser = JsParser::new();
        if let Some(tree) = parser.parse(js_source) {
            self.extract_bindings_from_js(tree.root_node(), js_source);
        }
    }

    /// JSのASTからテンプレートバインディングを抽出
    fn extract_bindings_from_js(&self, node: Node, source: &str) {
        if node.kind() == "call_expression" {
            if let Some(callee) = node.child_by_field_name("function") {
                if callee.kind() == "member_expression" {
                    if let Some(property) = callee.child_by_field_name("property") {
                        let method_name = self.node_text(property, source);
                        match method_name.as_str() {
                            "when" => self.extract_route_binding_js(node, callee, source),
                            "open" => self.extract_modal_binding_js(node, callee, source),
                            _ => {}
                        }
                    }
                }
            }
        }

        // 子ノードを再帰的に処理
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.extract_bindings_from_js(child, source);
        }
    }

    /// $routeProvider.when()からバインディングを抽出
    fn extract_route_binding_js(&self, node: Node, callee: Node, source: &str) {
        if let Some(object) = callee.child_by_field_name("object") {
            let obj_text = self.node_text(object, source);
            if !obj_text.ends_with("routeProvider") && !obj_text.ends_with("$routeProvider") {
                if object.kind() != "call_expression" {
                    return;
                }
            }
        }

        if let Some(args) = node.child_by_field_name("arguments") {
            if let Some(config_obj) = args.named_child(1) {
                if config_obj.kind() == "object" {
                    self.extract_template_binding_from_js_object(config_obj, source, BindingSource::RouteProvider);
                }
            }
        }
    }

    /// $uibModal.open()からバインディングを抽出
    fn extract_modal_binding_js(&self, node: Node, callee: Node, source: &str) {
        if let Some(object) = callee.child_by_field_name("object") {
            let obj_text = self.node_text(object, source);
            if !obj_text.ends_with("Modal") && !obj_text.ends_with("$uibModal") && !obj_text.ends_with("$modal") {
                return;
            }
        }

        if let Some(args) = node.child_by_field_name("arguments") {
            if let Some(config_obj) = args.named_child(0) {
                if config_obj.kind() == "object" {
                    self.extract_template_binding_from_js_object(config_obj, source, BindingSource::UibModal);
                }
            }
        }
    }

    /// JSオブジェクトからcontrollerとtemplateUrlを抽出
    fn extract_template_binding_from_js_object(&self, obj_node: Node, source: &str, binding_source: BindingSource) {
        let mut controller_name: Option<String> = None;
        let mut template_url: Option<String> = None;

        let mut cursor = obj_node.walk();
        for child in obj_node.children(&mut cursor) {
            if child.kind() == "pair" {
                if let Some(key) = child.child_by_field_name("key") {
                    let key_name = self.node_text(key, source);
                    let key_name = key_name.trim_matches(|c| c == '"' || c == '\'');

                    if let Some(value) = child.child_by_field_name("value") {
                        match key_name {
                            "controller" => {
                                if value.kind() == "string" {
                                    controller_name = Some(self.extract_string_value(value, source));
                                }
                            }
                            "templateUrl" => {
                                if value.kind() == "string" {
                                    template_url = Some(self.extract_string_value(value, source));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        if let (Some(controller), Some(template)) = (controller_name, template_url) {
            let binding = TemplateBinding {
                template_path: template,
                controller_name: controller,
                source: binding_source,
            };
            self.index.add_template_binding(binding);
        }
    }
}