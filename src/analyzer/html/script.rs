//! <script> tag JavaScript extraction

use tree_sitter::Node;

use super::HtmlAngularJsAnalyzer;

/// JavaScript code extracted from <script> tags in HTML files
#[derive(Debug, Clone)]
pub struct EmbeddedScript {
    pub source: String,
    pub line_offset: u32,
}

impl HtmlAngularJsAnalyzer {
    /// Extract JavaScript from <script> tags in HTML source
    pub fn extract_scripts(source: &str) -> Vec<EmbeddedScript> {
        let mut html_parser = super::parser::HtmlParser::new();
        let mut scripts = Vec::new();

        if let Some(tree) = html_parser.parse(source) {
            Self::collect_scripts_from_node(tree.root_node(), source, &mut scripts);
        }

        scripts
    }

    /// Extract JavaScript from a pre-parsed Tree
    pub fn extract_scripts_from_tree(root: Node, source: &str) -> Vec<EmbeddedScript> {
        let mut scripts = Vec::new();
        Self::collect_scripts_from_node(root, source, &mut scripts);
        scripts
    }

    /// Recursively collect <script> tag contents
    fn collect_scripts_from_node(node: Node, source: &str, scripts: &mut Vec<EmbeddedScript>) {
        if node.kind() == "script_element" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "raw_text" {
                    let js_source = source[child.byte_range()].to_string();
                    let line_offset = child.start_position().row as u32;
                    scripts.push(EmbeddedScript {
                        source: js_source,
                        line_offset,
                    });
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::collect_scripts_from_node(child, source, scripts);
        }
    }
}
