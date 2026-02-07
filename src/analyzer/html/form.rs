//! フォームバインディングの収集（Pass 2）

use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use crate::model::HtmlFormBinding;

use super::controller::ControllerScopeInfo;
use super::HtmlAngularJsAnalyzer;

impl HtmlAngularJsAnalyzer {
    /// フォームバインディングのみを収集（Pass 2用）
    /// ng-controllerスコープが確定した後に呼び出される
    pub(super) fn collect_form_bindings_from_tree(
        &self,
        node: Node,
        source: &str,
        uri: &Url,
        controller_stack: &mut Vec<ControllerScopeInfo>,
    ) {
        self.collect_form_bindings_recursive(node, source, uri, controller_stack);
    }

    /// フォームバインディング収集の再帰処理
    fn collect_form_bindings_recursive(
        &self,
        node: Node,
        source: &str,
        uri: &Url,
        controller_stack: &mut Vec<ControllerScopeInfo>,
    ) {
        if node.kind() == "element" {
            let mut added_controller = false;
            let scope_start_line = node.start_position().row as u32;
            let scope_end_line = node.end_position().row as u32;

            if let Some(start_tag) = self.find_child_by_kind(node, "start_tag") {
                // ng-controllerをチェックしてスタックに追加
                if let Some((controller_name, _alias)) =
                    self.get_ng_controller_attribute(start_tag, source)
                {
                    controller_stack.push(ControllerScopeInfo {
                        name: controller_name,
                        start_line: scope_start_line,
                        end_line: scope_end_line,
                    });
                    added_controller = true;
                }

                // <form name="x">からフォームバインディングを抽出
                if let Some(form_scope) =
                    self.extract_form_name_from_tag(start_tag, source, uri, scope_start_line, scope_end_line)
                {
                    // フォームはformタグの範囲ではなく、コントローラースコープ全体で参照可能
                    let (ctrl_start, ctrl_end) = controller_stack
                        .last()
                        .map(|c| (c.start_line, c.end_line))
                        .unwrap_or((0, u32::MAX));

                    let binding = HtmlFormBinding {
                        name: form_scope.name.clone(),
                        uri: uri.clone(),
                        scope_start_line: ctrl_start,
                        scope_end_line: ctrl_end,
                        name_start_line: form_scope.name_start_line,
                        name_start_col: form_scope.name_start_col,
                        name_end_line: form_scope.name_end_line,
                        name_end_col: form_scope.name_end_col,
                    };
                    self.index.html.add_html_form_binding(binding);
                }
            }

            // 子要素を再帰的に処理
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.collect_form_bindings_recursive(child, source, uri, controller_stack);
            }

            // このノードで追加したコントローラーをスタックから削除
            if added_controller {
                controller_stack.pop();
            }
        } else {
            // 子ノードを再帰的に処理
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.collect_form_bindings_recursive(child, source, uri, controller_stack);
            }
        }
    }
}
