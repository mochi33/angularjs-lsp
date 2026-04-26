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
