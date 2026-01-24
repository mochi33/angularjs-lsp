//! AngularJSディレクティブの定義

use phf::phf_set;

/// AngularJSディレクティブのセット（O(1)ルックアップ）
static NG_DIRECTIVE_SET: phf::Set<&'static str> = phf_set! {
    // データバインディング
    "ng-model", "data-ng-model",
    "ng-bind", "data-ng-bind",
    "ng-bind-html", "data-ng-bind-html",
    "ng-value", "data-ng-value",
    "ng-init", "data-ng-init",
    // 条件・繰り返し
    "ng-if", "data-ng-if",
    "ng-show", "data-ng-show",
    "ng-hide", "data-ng-hide",
    "ng-repeat", "data-ng-repeat",
    "ng-switch", "data-ng-switch",
    "ng-switch-when", "data-ng-switch-when",
    // スタイル・クラス
    "ng-class", "data-ng-class",
    "ng-style", "data-ng-style",
    // フォームバリデーション
    "ng-disabled", "data-ng-disabled",
    "ng-checked", "data-ng-checked",
    "ng-selected", "data-ng-selected",
    "ng-readonly", "data-ng-readonly",
    "ng-required", "data-ng-required",
    "ng-pattern", "data-ng-pattern",
    "ng-minlength", "data-ng-minlength",
    "ng-maxlength", "data-ng-maxlength",
    // イベントハンドラ
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
    // セレクト
    "ng-options", "data-ng-options",
    // href/src
    "ng-href", "data-ng-href",
    "ng-src", "data-ng-src",
    "ng-srcset", "data-ng-srcset",
    // ng-messages (Angular Messages module)
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

/// サポートするAngularJSディレクティブかどうかをチェック
pub fn is_ng_directive(attr_name: &str) -> bool {
    NG_DIRECTIVE_SET.contains(attr_name)
}
