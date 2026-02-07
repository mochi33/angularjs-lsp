//! カスタムディレクティブ参照の収集

use phf::phf_set;
use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::directives::is_ng_directive;
use super::HtmlAngularJsAnalyzer;
use crate::model::{DirectiveUsageType, HtmlDirectiveReference};

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

/// 標準HTML属性（カスタムディレクティブとして扱わない）
/// MDN HTML attribute reference: https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Attributes
static STANDARD_HTML_ATTRIBUTES: phf::Set<&'static str> = phf_set! {
    // Global attributes
    "accesskey", "anchor", "autocapitalize", "autocomplete", "autocorrect",
    "autofocus", "class", "contenteditable", "dir", "draggable",
    "enterkeyhint", "exportparts", "hidden", "id", "inert", "inputmode",
    "is", "itemid", "itemprop", "itemref", "itemscope", "itemtype",
    "lang", "nonce", "part", "popover", "role", "slot", "spellcheck",
    "style", "tabindex", "title", "translate", "virtualkeyboardpolicy",
    "writingsuggestions",

    // Element-specific attributes
    "abbr", "accept", "action", "align", "allow", "alpha", "alt", "as",
    "async", "autoplay", "background", "bgcolor", "border", "capture",
    "charset", "checked", "cite", "color", "colorspace", "cols", "colspan",
    "content", "controls", "coords", "crossorigin", "csp", "data",
    "datetime", "decoding", "default", "defer", "dirname", "disabled",
    "download", "enctype", "elementtiming", "fetchpriority", "for", "form",
    "formaction", "formenctype", "formmethod", "formnovalidate", "formtarget",
    "headers", "height", "high", "href", "hreflang", "ismap", "kind",
    "label", "language", "loading", "list", "loop", "low", "max",
    "maxlength", "media", "method", "min", "minlength", "multiple", "muted",
    "name", "novalidate", "open", "optimum", "pattern", "ping", "placeholder",
    "playsinline", "poster", "preload", "readonly", "referrerpolicy", "rel",
    "required", "reversed", "rows", "rowspan", "sandbox", "scope", "selected",
    "shape", "size", "sizes", "span", "src", "srcdoc", "srclang", "srcset",
    "start", "step", "summary", "target", "type", "usemap", "value", "width",
    "wrap",

    // Event handler attributes
    "onabort", "onanimationcancel", "onanimationend", "onanimationiteration",
    "onanimationstart", "onauxclick", "onbeforeinput", "onbeforematch",
    "onbeforetoggle", "onblur", "oncancel", "oncanplay", "oncanplaythrough",
    "onchange", "onclick", "onclose", "oncommand",
    "oncontentvisibilityautostatechange", "oncontextlost", "oncontextmenu",
    "oncontextrestored", "oncopy", "oncuechange", "oncut", "ondblclick",
    "ondrag", "ondragend", "ondragenter", "ondragleave", "ondragover",
    "ondragstart", "ondrop", "ondurationchange", "onemptied", "onended",
    "onerror", "onfocus", "onfocusin", "onfocusout", "onformdata",
    "onfullscreenchange", "onfullscreenerror", "ongesturechange",
    "ongestureend", "ongesturestart", "ongotpointercapture", "oninput",
    "oninvalid", "onkeydown", "onkeypress", "onkeyup", "onload",
    "onloadeddata", "onloadedmetadata", "onloadstart", "onlostpointercapture",
    "onmousedown", "onmouseenter", "onmouseleave", "onmousemove",
    "onmouseout", "onmouseover", "onmouseup", "onmousewheel", "onpaste",
    "onpause", "onplay", "onplaying", "onpointercancel", "onpointerdown",
    "onpointerenter", "onpointerleave", "onpointermove", "onpointerout",
    "onpointerover", "onpointerrawupdate", "onpointerup", "onprogress",
    "onratechange", "onreset", "onresize", "onscroll", "onscrollend",
    "onscrollsnapchange", "onscrollsnapchanging", "onsecuritypolicyviolation",
    "onseeked", "onseeking", "onselect", "onselectionchange", "onselectstart",
    "onslotchange", "onstalled", "onsubmit", "onsuspend", "ontimeupdate",
    "ontoggle", "ontouchcancel", "ontouchend", "ontouchmove", "ontouchstart",
    "ontransitioncancel", "ontransitionend", "ontransitionrun",
    "ontransitionstart", "onvolumechange", "onwaiting",
    "onwebkitmouseforcechanged", "onwebkitmouseforcedown",
    "onwebkitmouseforceup", "onwebkitmouseforcewillbegin", "onwheel",
};

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

    !STANDARD_HTML_ATTRIBUTES.contains(&lower)
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

    /// タグからディレクティブ参照を抽出
    ///
    /// ハイフンを含む要素名・属性名を全て潜在的なカスタムディレクティブとして登録する。
    /// 定義の有無は定義ジャンプ時にチェックするため、解析順序に依存しない。
    fn extract_directive_from_tag(&self, tag_node: Node, source: &str, uri: &Url) {
        // 1. 要素名としてのディレクティブをチェック
        if let Some(tag_name_node) = self.find_child_by_kind(tag_node, "tag_name") {
            let tag_name = self.node_text(tag_name_node, source);

            // カスタム要素の可能性があるかチェック
            if is_potential_custom_element(&tag_name) {
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
                self.index.html.add_html_directive_reference(reference);
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
                        self.index.html.add_html_directive_reference(reference);
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
