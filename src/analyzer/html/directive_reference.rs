//! カスタムディレクティブ参照の収集

use phf::phf_set;
use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::directives::{is_ng_directive, is_standard_html_attribute};
use super::HtmlAngularJsAnalyzer;
use crate::model::{
    DirectiveUsageType, HtmlComponentAttribute, HtmlComponentUsage, HtmlDirectiveReference,
};

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

/// 標準HTML要素（カスタムディレクティブとして扱わない）
static STANDARD_HTML_ELEMENTS: phf::Set<&'static str> = phf_set! {
    "div", "span", "p", "a", "img", "button", "input", "form",
    "table", "tr", "td", "th", "thead", "tbody", "tfoot",
    "ul", "ol", "li", "dl", "dt", "dd",
    "h1", "h2", "h3", "h4", "h5", "h6",
    "header", "footer", "nav", "main", "section", "article", "aside",
    "label", "select", "option", "optgroup", "textarea", "fieldset", "legend",
    "iframe", "video", "audio", "source", "canvas", "svg",
    "br", "hr", "pre", "code", "blockquote", "em", "strong", "small",
    "script", "link", "meta", "style", "head", "body", "html", "title",
};

/// 名前がカスタムディレクティブの可能性があるかチェック（属性用）
/// - aria-* と data-* は標準HTML属性パターンなので除外
/// - ハイフンを含む場合はカスタムディレクティブの可能性あり
/// - ハイフンなしでも標準HTML属性以外はカスタムディレクティブの可能性あり
fn is_potential_custom_directive(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();

    // aria-* と data-* は標準HTML属性パターン
    if lower.starts_with("aria-") || lower.starts_with("data-") {
        return false;
    }

    // ハイフンを含む場合はカスタムディレクティブの可能性あり
    if name.contains('-') {
        return true;
    }

    !is_standard_html_attribute(&lower)
}

/// 名前がカスタム要素の可能性があるかチェック（要素名用）
fn is_potential_custom_element(name: &str) -> bool {
    if name.contains('-') {
        return true;
    }
    !STANDARD_HTML_ELEMENTS.contains(&name.to_ascii_lowercase())
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

    /// タグからディレクティブ参照を抽出する。
    ///
    /// 2 種類の情報を 1 パスで集める:
    /// - **DirectiveReference** (要素名 + 属性名): 既存のジャンプ・ホバー用。
    ///   ハイフンを含む要素名・属性名を全て潜在的なカスタムディレクティブとして登録する。
    ///   定義の有無は定義ジャンプ時にチェックするため、解析順序に依存しない。
    /// - **HtmlComponentUsage** (要素単位 + 全属性リスト): component bindings 診断
    ///   (#64) 用。要素名がカスタム要素の可能性がある場合のみ作る。
    ///
    /// 「ディレクティブ抽出」と「コンポーネント使用記録」は AST を 2 度走査せずに
    /// 同一ループで済ませるため、ここで束ねている。各ステップは私的ヘルパー
    /// (`record_element_directive_reference` / `record_attribute_directive_reference`)
    /// に切り分けて責務を見やすくしている。
    fn extract_directive_from_tag(&self, tag_node: Node, source: &str, uri: &Url) {
        // 1. 要素名 → DirectiveReference + (任意で) 空の HtmlComponentUsage 雛形
        let mut component_usage = self.record_element_directive_reference(tag_node, source, uri);

        // 2. 各属性 → DirectiveReference + (usage 雛形があれば) HtmlComponentAttribute
        let mut cursor = tag_node.walk();
        for child in tag_node.children(&mut cursor) {
            if child.kind() != "attribute" {
                continue;
            }
            self.record_attribute_directive_reference(
                child,
                source,
                uri,
                component_usage.as_mut(),
            );
        }

        // 3. 要素ベースのコンポーネント使用箇所を確定
        if let Some(usage) = component_usage {
            self.index.html.add_component_usage(usage);
        }
    }

    /// 要素名トークンを処理し、必要なら `HtmlDirectiveReference` を登録、
    /// さらに後続の属性ループ用の `HtmlComponentUsage` 雛形を返す。
    ///
    /// 標準 HTML 要素 (`div`, `span` 等) は雛形を作らず `None` を返す。
    fn record_element_directive_reference(
        &self,
        tag_node: Node,
        source: &str,
        uri: &Url,
    ) -> Option<HtmlComponentUsage> {
        let tag_name_node = self.find_child_by_kind(tag_node, "tag_name")?;
        let tag_name = self.node_text(tag_name_node, source);

        if !is_potential_custom_element(&tag_name) {
            return None;
        }

        let camel_name = kebab_to_camel_case(&tag_name);
        let start = tag_name_node.start_position();
        let end = tag_name_node.end_position();
        let element_start_col = self.byte_col_to_utf16_col(source, start.row, start.column);
        let element_end_col = self.byte_col_to_utf16_col(source, end.row, end.column);

        self.index.html.add_html_directive_reference(HtmlDirectiveReference {
            directive_name: camel_name.clone(),
            uri: uri.clone(),
            start_line: start.row as u32,
            start_col: element_start_col,
            end_line: end.row as u32,
            end_col: element_end_col,
            usage_type: DirectiveUsageType::Element,
        });

        Some(HtmlComponentUsage {
            component_name: camel_name,
            uri: uri.clone(),
            element_start_line: start.row as u32,
            element_start_col,
            element_end_line: end.row as u32,
            element_end_col,
            attributes: Vec::new(),
        })
    }

    /// 属性 1 つを処理する:
    /// - `usage` 雛形があれば `HtmlComponentAttribute` を必ず追加 (#64 診断用)
    /// - ビルトイン (`ng-*`) はディレクティブ参照を作らない
    /// - カスタムディレクティブの可能性があるなら `HtmlDirectiveReference` を登録
    fn record_attribute_directive_reference(
        &self,
        attribute_node: Node,
        source: &str,
        uri: &Url,
        usage: Option<&mut HtmlComponentUsage>,
    ) {
        let Some(name_node) = self.find_child_by_kind(attribute_node, "attribute_name") else {
            return;
        };
        let attr_name = self.node_text(name_node, source);

        // data- プレフィックスを除去
        let normalized_attr = if attr_name.starts_with("data-") {
            &attr_name[5..]
        } else {
            &attr_name
        };

        let start = name_node.start_position();
        let end = name_node.end_position();
        let attr_start_col = self.byte_col_to_utf16_col(source, start.row, start.column);
        let attr_end_col = self.byte_col_to_utf16_col(source, end.row, end.column);

        // コンポーネント使用箇所には全属性を記録 (診断側でフィルタ)
        if let Some(usage) = usage {
            let camel_name = kebab_to_camel_case(normalized_attr);
            usage.attributes.push(HtmlComponentAttribute {
                name: attr_name.to_string(),
                camel_name,
                start_line: start.row as u32,
                start_col: attr_start_col,
                end_line: end.row as u32,
                end_col: attr_end_col,
            });
        }

        // ビルトイン ng-* / data-ng-* はディレクティブとして登録しない
        if is_ng_directive(&attr_name) {
            return;
        }

        if is_potential_custom_directive(normalized_attr) {
            let camel_name = kebab_to_camel_case(normalized_attr);
            self.index.html.add_html_directive_reference(HtmlDirectiveReference {
                directive_name: camel_name,
                uri: uri.clone(),
                start_line: start.row as u32,
                start_col: attr_start_col,
                end_line: end.row as u32,
                end_col: attr_end_col,
                usage_type: DirectiveUsageType::Attribute,
            });
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
        // ハイフン付きカスタムディレクティブ
        assert!(is_potential_custom_directive("my-directive"));
        assert!(is_potential_custom_directive("custom-element"));

        // ハイフンなしカスタムディレクティブ
        assert!(is_potential_custom_directive("strdigit"));
        assert!(is_potential_custom_directive("myDirective"));

        // 標準HTML属性（グローバル属性）
        assert!(!is_potential_custom_directive("class"));
        assert!(!is_potential_custom_directive("id"));
        assert!(!is_potential_custom_directive("style"));
        assert!(!is_potential_custom_directive("tabindex"));
        assert!(!is_potential_custom_directive("contenteditable"));

        // 標準HTML属性（要素固有属性）
        assert!(!is_potential_custom_directive("href"));
        assert!(!is_potential_custom_directive("src"));
        assert!(!is_potential_custom_directive("placeholder"));
        assert!(!is_potential_custom_directive("readonly"));

        // イベントハンドラ属性
        assert!(!is_potential_custom_directive("onclick"));
        assert!(!is_potential_custom_directive("onchange"));
        assert!(!is_potential_custom_directive("onmouseover"));
        assert!(!is_potential_custom_directive("onkeydown"));

        // aria-* 属性パターン
        assert!(!is_potential_custom_directive("aria-label"));
        assert!(!is_potential_custom_directive("aria-hidden"));
        assert!(!is_potential_custom_directive("aria-describedby"));
        assert!(!is_potential_custom_directive("aria-expanded"));

        // data-* 属性パターン
        assert!(!is_potential_custom_directive("data-id"));
        assert!(!is_potential_custom_directive("data-value"));
        assert!(!is_potential_custom_directive("data-custom-attr"));
    }

    #[test]
    fn test_is_potential_custom_element() {
        assert!(is_potential_custom_element("my-component"));
        assert!(is_potential_custom_element("customElement"));
        assert!(!is_potential_custom_element("div"));
        assert!(!is_potential_custom_element("span"));
    }
}
