//! カスタムディレクティブ参照の収集

use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::directives::is_ng_directive;
use super::HtmlAngularJsAnalyzer;
use crate::index::{DirectiveUsageType, HtmlDirectiveReference};

/// kebab-case を camelCase に変換
/// 例: "my-directive" -> "myDirective"
fn kebab_to_camel_case(name: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;

    for c in name.chars() {
        if c == '-' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }

    result
}

/// 名前がカスタムディレクティブの可能性があるかチェック
/// - ハイフンを含む（Web Components / カスタム要素の規約）
fn is_potential_custom_directive(name: &str) -> bool {
    name.contains('-')
}

impl HtmlAngularJsAnalyzer {
    /// カスタムディレクティブ参照を収集
    pub(super) fn collect_directive_references(&self, node: Node, source: &str, uri: &Url) {
        self.collect_directive_references_impl(node, source, uri);
    }

    fn collect_directive_references_impl(&self, node: Node, source: &str, uri: &Url) {
        // 要素ノードの場合
        if node.kind() == "element" {
            if let Some(start_tag) = self.find_child_by_kind(node, "start_tag") {
                self.extract_directive_from_tag(start_tag, source, uri);
            }
        }

        // 自己終了タグの場合
        if node.kind() == "self_closing_tag" {
            self.extract_directive_from_tag(node, source, uri);
        }

        // 子ノードを再帰的に処理
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.collect_directive_references_impl(child, source, uri);
        }
    }

    /// タグからディレクティブ参照を抽出
    ///
    /// ハイフンを含む要素名・属性名を全て潜在的なカスタムディレクティブとして登録する。
    /// 定義の有無は定義ジャンプ時にチェックするため、解析順序に依存しない。
    fn extract_directive_from_tag(&self, tag_node: Node, source: &str, uri: &Url) {
        // 1. 要素名としてのディレクティブをチェック
        if let Some(tag_name_node) = self.find_child_by_kind(tag_node, "tag_name") {
            let tag_name = self.node_text(tag_name_node, source);

            // カスタムディレクティブの可能性があるかチェック
            if is_potential_custom_directive(&tag_name) {
                let camel_name = kebab_to_camel_case(&tag_name);
                let start = tag_name_node.start_position();
                let end = tag_name_node.end_position();

                let reference = HtmlDirectiveReference {
                    directive_name: camel_name,
                    uri: uri.clone(),
                    start_line: start.row as u32,
                    start_col: self.byte_col_to_utf16_col(source, start.row, start.column),
                    end_line: end.row as u32,
                    end_col: self.byte_col_to_utf16_col(source, end.row, end.column),
                    usage_type: DirectiveUsageType::Element,
                };
                self.index.add_html_directive_reference(reference);
            }
        }

        // 2. 属性としてのディレクティブをチェック
        let mut cursor = tag_node.walk();
        for child in tag_node.children(&mut cursor) {
            if child.kind() == "attribute" {
                if let Some(name_node) = self.find_child_by_kind(child, "attribute_name") {
                    let attr_name = self.node_text(name_node, source);

                    // data- プレフィックスを除去
                    let normalized_attr = if attr_name.starts_with("data-") {
                        &attr_name[5..]
                    } else {
                        &attr_name
                    };

                    // ビルトインng-*ディレクティブは除外
                    if is_ng_directive(&attr_name) {
                        continue;
                    }

                    // カスタムディレクティブの可能性があるかチェック
                    if is_potential_custom_directive(normalized_attr) {
                        let camel_name = kebab_to_camel_case(normalized_attr);
                        let start = name_node.start_position();
                        let end = name_node.end_position();

                        let reference = HtmlDirectiveReference {
                            directive_name: camel_name,
                            uri: uri.clone(),
                            start_line: start.row as u32,
                            start_col: self.byte_col_to_utf16_col(
                                source,
                                start.row,
                                start.column,
                            ),
                            end_line: end.row as u32,
                            end_col: self.byte_col_to_utf16_col(source, end.row, end.column),
                            usage_type: DirectiveUsageType::Attribute,
                        };
                        self.index.add_html_directive_reference(reference);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kebab_to_camel_case() {
        assert_eq!(kebab_to_camel_case("my-directive"), "myDirective");
        assert_eq!(kebab_to_camel_case("my-custom-element"), "myCustomElement");
        assert_eq!(kebab_to_camel_case("simple"), "simple");
        assert_eq!(kebab_to_camel_case("a-b-c"), "aBC");
    }

    #[test]
    fn test_is_potential_custom_directive() {
        assert!(is_potential_custom_directive("my-directive"));
        assert!(is_potential_custom_directive("custom-element"));
        assert!(!is_potential_custom_directive("div"));
        assert!(!is_potential_custom_directive("span"));
    }
}
