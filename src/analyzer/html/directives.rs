//! AngularJS directive definitions

use phf::phf_set;

use crate::index::Index;
use crate::model::SymbolKind;
use crate::util::kebab_to_camel;

/// AngularJS directive set (O(1) lookup)
static NG_DIRECTIVE_SET: phf::Set<&'static str> = phf_set! {
    // Data binding
    "ng-model", "data-ng-model",
    "ng-bind", "data-ng-bind",
    "ng-bind-html", "data-ng-bind-html",
    "ng-value", "data-ng-value",
    "ng-init", "data-ng-init",
    // Conditionals & loops
    "ng-if", "data-ng-if",
    "ng-show", "data-ng-show",
    "ng-hide", "data-ng-hide",
    "ng-repeat", "data-ng-repeat",
    "ng-switch", "data-ng-switch",
    "ng-switch-when", "data-ng-switch-when",
    // Style & class
    "ng-class", "data-ng-class",
    "ng-style", "data-ng-style",
    // Form validation
    "ng-disabled", "data-ng-disabled",
    "ng-checked", "data-ng-checked",
    "ng-selected", "data-ng-selected",
    "ng-readonly", "data-ng-readonly",
    "ng-required", "data-ng-required",
    "ng-pattern", "data-ng-pattern",
    "ng-minlength", "data-ng-minlength",
    "ng-maxlength", "data-ng-maxlength",
    // Event handlers
    "ng-click", "data-ng-click",
    "ng-dblclick", "data-ng-dblclick",
    "ng-change", "data-ng-change",
    "ng-submit", "data-ng-submit",
    "ng-blur", "data-ng-blur",
    "ng-focus", "data-ng-focus",
    "ng-keydown", "data-ng-keydown",
    "ng-keyup", "data-ng-keyup",
    "ng-keypress", "data-ng-keypress",
    "ng-mousedown", "data-ng-mousedown",
    "ng-mouseup", "data-ng-mouseup",
    "ng-mouseenter", "data-ng-mouseenter",
    "ng-mouseleave", "data-ng-mouseleave",
    "ng-mousemove", "data-ng-mousemove",
    "ng-mouseover", "data-ng-mouseover",
    "ng-copy", "data-ng-copy",
    "ng-cut", "data-ng-cut",
    "ng-paste", "data-ng-paste",
    // Select
    "ng-options", "data-ng-options",
    // href/src
    "ng-href", "data-ng-href",
    "ng-src", "data-ng-src",
    "ng-srcset", "data-ng-srcset",
    // ng-messages
    "ng-messages", "data-ng-messages",
    "ng-message", "data-ng-message",
    "ng-messages-include", "data-ng-messages-include",
    // angular-file-upload (ngf-*)
    "ngf-select", "ngf-drop", "ngf-drop-available",
    "ngf-multiple", "ngf-keep", "ngf-keep-distinct",
    "ngf-accept", "ngf-capture", "ngf-pattern",
    "ngf-validate", "ngf-drag-over-class",
    "ngf-model-options", "ngf-resize", "ngf-thumbnail",
    "ngf-max-size", "ngf-min-size", "ngf-max-height",
    "ngf-min-height", "ngf-max-width", "ngf-min-width",
    "ngf-max-duration", "ngf-min-duration",
    "ngf-max-files", "ngf-min-files",
    "ngf-change", "ngf-fix-orientation",
    // UI Bootstrap (uib-*)
    "uib-tooltip", "uib-tooltip-html", "uib-tooltip-template",
    "uib-popover", "uib-popover-html", "uib-popover-template",
    "uib-modal", "uib-typeahead", "uib-datepicker",
    "uib-datepicker-popup", "uib-timepicker",
    "uib-accordion", "uib-accordion-group",
    "uib-collapse", "uib-dropdown", "uib-dropdown-toggle",
    "uib-pagination", "uib-pager", "uib-progressbar",
    "uib-rating", "uib-tabset", "uib-tab",
    "uib-alert", "uib-carousel", "uib-slide",
    "uib-btn-checkbox", "uib-btn-radio",
    // tooltip/popover options
    "tooltip-placement", "tooltip-trigger", "tooltip-append-to-body",
    "popover-placement", "popover-trigger", "popover-append-to-body",
};

/// Check if attribute name is a supported AngularJS directive
pub fn is_ng_directive(attr_name: &str) -> bool {
    NG_DIRECTIVE_SET.contains(attr_name)
}

/// 標準 HTML 属性 (カスタムディレクティブ / コンポーネント binding として扱わない)
///
/// MDN HTML attribute reference: https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Attributes
///
/// `directive_reference.rs` の custom directive 判定と、`diagnostics.rs` の
/// component bindings 漏れ判定 (#64) で共有される。重複定義回避のためここに集約。
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

/// 標準 HTML 属性かどうかを判定 (lowercase 比較)。
///
/// 呼び出し側はあらかじめ小文字化しておくこと (HTML 属性は case-insensitive だが
/// この set 自体は lowercase で持っている)。
pub fn is_standard_html_attribute(attr_name_lower: &str) -> bool {
    STANDARD_HTML_ATTRIBUTES.contains(attr_name_lower)
}

/// 値が **Angular 式ではなくリテラル文字列 / 正規表現 / 補間テンプレート**
/// として解釈されるディレクティブ集合。
///
/// これらはディレクティブ自体は AngularJS が認識するが、属性値はスコープ参照
/// として解析するべきではない:
///
/// **literal string match 系:**
/// - `ng-message="required"` — `$error.required` の検証キー名
/// - `ng-messages-include="error-messages.html"` — テンプレート URL
/// - `ng-switch-when="red"` — `ng-switch` の値との string match (case ラベル)
///
/// **regex literal 系:**
/// - `ng-pattern="/^\d+$/"` — 値は正規表現リテラルまたは正規表現文字列。
///   AngularJS が `$eval` の結果を `RegExp` として使う。一般的な使用は
///   インライン正規表現で scope 変数参照ではない
///
/// **interpolation-only template 系:**
/// - `ng-src="{{vm.imageUrl}}"` — 値は補間テンプレート (`{{}}` を含む文字列)。
///   AngularJS は補間後の文字列を src 属性に設定する。bare expression として
///   `ng-src="vm.imageUrl"` と書いても展開されないので、補間のみ抽出すれば足りる
///
/// 参考: AngularJS source (`ngSwitchWhenDirective`) は `attrs.ngSwitchWhen` を
/// `$eval` せず literal として `ctrl.cases['!' + value]` のキーに使っている。
static LITERAL_VALUE_DIRECTIVE_SET: phf::Set<&'static str> = phf_set! {
    // literal string match
    "ng-message", "data-ng-message",
    "ng-messages-include", "data-ng-messages-include",
    "ng-switch-when", "data-ng-switch-when",
    // regex literal
    "ng-pattern", "data-ng-pattern",
    // interpolation-only template
    "ng-src", "data-ng-src",
};

/// 属性値が Angular 式ではなくリテラル文字列として解釈されるディレクティブか判定
pub fn is_literal_value_directive(attr_name: &str) -> bool {
    LITERAL_VALUE_DIRECTIVE_SET.contains(attr_name)
}

/// 属性値を Angular 式として解析すべきか判定する。
///
/// 以下のいずれかに当てはまる場合 `true` を返す:
/// 1. ビルトイン or 既知ライブラリの ng-* / uib-* / ngf-* ディレクティブ
///    (`is_ng_directive` の判定。`data-` 接頭辞は揺らぎを吸収)
/// 2. JS 側で `.directive('name', ...)` 登録された custom directive
///    (kebab-case → camelCase で `SymbolKind::Directive` を index に検索)
/// 3. `element_name` が `.component('name', ...)` 登録された component で、
///    かつ属性名がその component の `bindings` の名前と一致
///    (`SymbolKind::ComponentBinding` で `componentName.bindingName` を検索)
///
/// `element_name` は属性が属する要素のタグ名 (kebab-case)。
/// `None` の場合は判定 (1) と (2) のみ行う (component bindings は判定不能)。
pub fn is_directive_attribute(
    attr_name: &str,
    element_name: Option<&str>,
    index: &Index,
) -> bool {
    // 1. ビルトイン/既知ライブラリ
    if is_ng_directive(attr_name) {
        return true;
    }

    // data- 接頭辞を剥がしてから index 検索
    let stripped = attr_name.strip_prefix("data-").unwrap_or(attr_name);
    let camel = kebab_to_camel(stripped);

    // 2. custom directive
    if index
        .definitions
        .has_definition_of_kind(&camel, SymbolKind::Directive)
    {
        return true;
    }

    // 3. component binding (要素名が必要)
    if let Some(elem) = element_name {
        let elem_camel = kebab_to_camel(elem);
        if index
            .definitions
            .has_definition_of_kind(&elem_camel, SymbolKind::Component)
        {
            let binding_name = format!("{}.{}", elem_camel, camel);
            if index
                .definitions
                .has_definition_of_kind(&binding_name, SymbolKind::ComponentBinding)
            {
                return true;
            }
        }
    }

    false
}
