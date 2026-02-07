//! <script>タグ内のJavaScript抽出

use tree_sitter::Node;

use super::HtmlParser;
use super::HtmlAngularJsAnalyzer;

/// HTMLファイル内の<script>タグから抽出されたJavaScriptコード
#[derive(Debug, Clone)]
pub struct EmbeddedScript {
    /// JavaScriptソースコード
    pub source: String,
    /// scriptタグの開始行（行番号オフセット）
    pub line_offset: u32,
}

impl HtmlAngularJsAnalyzer {
    /// HTMLソースから<script>タグ内のJavaScriptを抽出
    ///
    /// server.rsのJS Pass 1/2から呼び出される
    pub fn extract_scripts(source: &str) -> Vec<EmbeddedScript> {
        let mut parser = HtmlParser::new();
        let mut scripts = Vec::new();

        if let Some(tree) = parser.parse(source) {
            Self::collect_scripts_from_node(tree.root_node(), source, &mut scripts);
        }

        scripts
    }

    /// 事前にパースしたTreeから<script>タグ内のJavaScriptを抽出
    pub fn extract_scripts_from_tree(root: Node, source: &str) -> Vec<EmbeddedScript> {
        let mut scripts = Vec::new();
        Self::collect_scripts_from_node(root, source, &mut scripts);
        scripts
    }

    /// ノードを再帰的に走査して<script>タグを収集
    fn collect_scripts_from_node(node: Node, source: &str, scripts: &mut Vec<EmbeddedScript>) {
        if node.kind() == "script_element" {
            // <script>タグの内容を取得
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

        // 子ノードを再帰的に処理
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::collect_scripts_from_node(child, source, scripts);
        }
    }
}

