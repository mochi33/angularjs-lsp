//! ng-includeバインディングの収集（Pass 1.5）

use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use crate::model::{
    HtmlLocalVariableSource, InheritedFormBinding, InheritedLocalVariable,
    NgIncludeBinding, NgViewBinding,
};

use super::variable_parser::is_valid_identifier;

use super::controller::ControllerScopeInfo;
use super::variable_parser::{parse_ng_init_expression, parse_ng_repeat_expression};
use super::HtmlAngularJsAnalyzer;

/// ローカル変数スコープ情報（収集時に使用）
#[derive(Clone, Debug)]
struct LocalVariableScope {
    name: String,
    source: HtmlLocalVariableSource,
    /// 定義元のURI（継承されたものは親のURI、現在のファイルのものは現在のURI）
    uri: Url,
    scope_start_line: u32,
    scope_end_line: u32,
    name_start_line: u32,
    name_start_col: u32,
    name_end_line: u32,
    name_end_col: u32,
}

/// フォームバインディングスコープ情報（収集時に使用）
#[derive(Clone, Debug)]
pub(super) struct FormBindingScope {
    pub(super) name: String,
    /// 定義元のURI（継承されたものは親のURI、現在のファイルのものは現在のURI）
    pub(super) uri: Url,
    pub(super) scope_start_line: u32,
    pub(super) scope_end_line: u32,
    pub(super) name_start_line: u32,
    pub(super) name_start_col: u32,
    pub(super) name_end_line: u32,
    pub(super) name_end_col: u32,
    /// このformが属するコントローラースタックの深さ
    /// コントローラースコープ終了時にまとめてpopするために使用
    pub(super) controller_depth: usize,
}

impl HtmlAngularJsAnalyzer {
    /// ng-includeバインディングを収集（Pass 1.5用）
    /// 継承チェーンを考慮してコントローラー、ローカル変数、フォームバインディングを継承
    pub(super) fn collect_ng_include_bindings_from_tree(
        &self,
        node: Node,
        source: &str,
        uri: &Url,
        controller_stack: &mut Vec<ControllerScopeInfo>,
    ) {
        // 継承されたローカル変数を初期スタックに追加
        let inherited_local_vars = self.index.templates.get_inherited_local_variables_for_template(uri);
        let mut local_var_stack: Vec<LocalVariableScope> = inherited_local_vars
            .into_iter()
            .map(|v| LocalVariableScope {
                name: v.name,
                source: v.source,
                uri: v.uri, // 元の定義元URIを保持
                scope_start_line: 0,
                scope_end_line: u32::MAX,
                name_start_line: v.name_start_line,
                name_start_col: v.name_start_col,
                name_end_line: v.name_end_line,
                name_end_col: v.name_end_col,
            })
            .collect();

        // 継承されたフォームバインディングを初期スタックに追加
        let inherited_forms = self.index.templates.get_inherited_form_bindings_for_template(uri);
        let mut form_binding_stack: Vec<FormBindingScope> = inherited_forms
            .into_iter()
            .map(|f| FormBindingScope {
                name: f.name,
                uri: f.uri, // 元の定義元URIを保持
                scope_start_line: 0,
                scope_end_line: u32::MAX,
                name_start_line: f.name_start_line,
                name_start_col: f.name_start_col,
                name_end_line: f.name_end_line,
                name_end_col: f.name_end_col,
                controller_depth: 0, // 継承されたものは最上位スコープ
            })
            .collect();

        self.collect_ng_include_bindings_impl(
            node,
            source,
            uri,
            controller_stack,
            &mut local_var_stack,
            &mut form_binding_stack,
        );
    }

    /// ng-includeバインディング収集の実装
    fn collect_ng_include_bindings_impl(
        &self,
        node: Node,
        source: &str,
        uri: &Url,
        controller_stack: &mut Vec<ControllerScopeInfo>,
        local_var_stack: &mut Vec<LocalVariableScope>,
        form_binding_stack: &mut Vec<FormBindingScope>,
    ) {
        // element または self_closing_tag を処理
        let (is_element, tag_node) = if node.kind() == "element" {
            (true, self.find_child_by_kind(node, "start_tag"))
        } else if node.kind() == "self_closing_tag" {
            (false, Some(node))
        } else {
            (false, None)
        };

        if let Some(start_tag) = tag_node {
            let mut added_controller = false;
            let mut added_local_vars: Vec<String> = Vec::new();

            let scope_start_line = node.start_position().row as u32;
            let scope_end_line = node.end_position().row as u32;

            // ng-controllerをチェック（スタック管理用、登録は不要）
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

            // ng-repeatからローカル変数を抽出
            if let Some(vars) =
                self.extract_local_vars_from_ng_repeat(start_tag, source, uri, scope_start_line, scope_end_line)
            {
                for var in vars {
                    added_local_vars.push(var.name.clone());
                    local_var_stack.push(var);
                }
            }

            // ng-initからローカル変数を抽出
            if let Some(vars) =
                self.extract_local_vars_from_ng_init(start_tag, source, uri, scope_start_line, scope_end_line)
            {
                for var in vars {
                    added_local_vars.push(var.name.clone());
                    local_var_stack.push(var);
                }
            }

            // <form name="x">からフォームバインディングを抽出（スタック管理用）
            if let Some(mut form_scope) =
                self.extract_form_name_from_tag(start_tag, source, uri, scope_start_line, scope_end_line)
            {
                let (ctrl_start, ctrl_end) = controller_stack
                    .last()
                    .map(|c| (c.start_line, c.end_line))
                    .unwrap_or((0, u32::MAX));
                form_scope.scope_start_line = ctrl_start;
                form_scope.scope_end_line = ctrl_end;
                // formはコントローラースコープに属するため、コントローラースタックの深さを記録
                // コントローラースコープ終了時にまとめてpopする
                form_scope.controller_depth = controller_stack.len();
                form_binding_stack.push(form_scope);
            }

            // ng-includeをチェック
            if let Some(template_path) = self.get_ng_include_attribute(start_tag, source) {
                let resolved_filename = crate::util::resolve_relative_path(uri, &template_path);

                // ローカル変数を継承情報に変換
                // 元の定義元URIを保持（継承チェーンを通じて伝播するため）
                let inherited_local_variables: Vec<InheritedLocalVariable> = local_var_stack
                    .iter()
                    .map(|v| InheritedLocalVariable {
                        name: v.name.clone(),
                        source: v.source.clone(),
                        uri: v.uri.clone(), // 元の定義元URIを保持
                        scope_start_line: v.scope_start_line,
                        scope_end_line: v.scope_end_line,
                        name_start_line: v.name_start_line,
                        name_start_col: v.name_start_col,
                        name_end_line: v.name_end_line,
                        name_end_col: v.name_end_col,
                    })
                    .collect();

                // フォームバインディングを継承情報に変換
                // 元の定義元URIを保持（継承チェーンを通じて伝播するため）
                let inherited_form_bindings: Vec<InheritedFormBinding> = form_binding_stack
                    .iter()
                    .map(|f| InheritedFormBinding {
                        name: f.name.clone(),
                        uri: f.uri.clone(), // 元の定義元URIを保持
                        scope_start_line: f.scope_start_line,
                        scope_end_line: f.scope_end_line,
                        name_start_line: f.name_start_line,
                        name_start_col: f.name_start_col,
                        name_end_line: f.name_end_line,
                        name_end_col: f.name_end_col,
                    })
                    .collect();

                // コントローラー名を収集
                let inherited_controller_names: Vec<String> =
                    controller_stack.iter().map(|c| c.name.clone()).collect();

                let binding = NgIncludeBinding {
                    parent_uri: uri.clone(),
                    template_path,
                    resolved_filename,
                    line: scope_start_line,
                    inherited_controllers: inherited_controller_names,
                    inherited_local_variables,
                    inherited_form_bindings,
                };
                self.index.templates.add_ng_include_binding(binding);
            }

            // ng-viewをチェック（<ng-view>, <div ng-view>, <div data-ng-view>）
            if self.is_ng_view_element(start_tag, source) {
                // ローカル変数を継承情報に変換
                let inherited_local_variables: Vec<InheritedLocalVariable> = local_var_stack
                    .iter()
                    .map(|v| InheritedLocalVariable {
                        name: v.name.clone(),
                        source: v.source.clone(),
                        uri: v.uri.clone(),
                        scope_start_line: v.scope_start_line,
                        scope_end_line: v.scope_end_line,
                        name_start_line: v.name_start_line,
                        name_start_col: v.name_start_col,
                        name_end_line: v.name_end_line,
                        name_end_col: v.name_end_col,
                    })
                    .collect();

                // フォームバインディングを継承情報に変換
                let inherited_form_bindings: Vec<InheritedFormBinding> = form_binding_stack
                    .iter()
                    .map(|f| InheritedFormBinding {
                        name: f.name.clone(),
                        uri: f.uri.clone(),
                        scope_start_line: f.scope_start_line,
                        scope_end_line: f.scope_end_line,
                        name_start_line: f.name_start_line,
                        name_start_col: f.name_start_col,
                        name_end_line: f.name_end_line,
                        name_end_col: f.name_end_col,
                    })
                    .collect();

                // コントローラー名を収集
                let inherited_controller_names: Vec<String> =
                    controller_stack.iter().map(|c| c.name.clone()).collect();

                let binding = NgViewBinding {
                    parent_uri: uri.clone(),
                    line: scope_start_line,
                    inherited_controllers: inherited_controller_names,
                    inherited_local_variables,
                    inherited_form_bindings,
                };
                self.index.templates.add_ng_view_binding(binding);
            }

            // 子要素を再帰的に処理（elementの場合のみ）
            if is_element {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    self.collect_ng_include_bindings_impl(
                        child,
                        source,
                        uri,
                        controller_stack,
                        local_var_stack,
                        form_binding_stack,
                    );
                }

                // スタックから削除
                if added_controller {
                    // コントローラースコープ終了時、このスコープに属するformバインディングをすべてpop
                    // formはコントローラースコープに登録されるため、DOM構造ではなくコントローラースコープに従う
                    let depth_after_pop = controller_stack.len() - 1;
                    form_binding_stack.retain(|f| f.controller_depth <= depth_after_pop);
                    controller_stack.pop();
                }
                for var_name in added_local_vars {
                    if let Some(pos) = local_var_stack.iter().rposition(|v| v.name == var_name) {
                        local_var_stack.remove(pos);
                    }
                }
            }
        } else {
            // tag_nodeがない場合は子要素のみ再帰的に処理
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.collect_ng_include_bindings_impl(
                    child,
                    source,
                    uri,
                    controller_stack,
                    local_var_stack,
                    form_binding_stack,
                );
            }
        }
    }

    /// ng-repeatからローカル変数を抽出
    fn extract_local_vars_from_ng_repeat(
        &self,
        start_tag: Node,
        source: &str,
        uri: &Url,
        scope_start_line: u32,
        scope_end_line: u32,
    ) -> Option<Vec<LocalVariableScope>> {
        let mut cursor = start_tag.walk();
        for child in start_tag.children(&mut cursor) {
            if child.kind() == "attribute" {
                if let Some(name_node) = self.find_child_by_kind(child, "attribute_name") {
                    let attr_name = self.node_text(name_node, source);

                    if attr_name == "ng-repeat" || attr_name == "data-ng-repeat" {
                        if let Some(value_node) =
                            self.find_child_by_kind(child, "quoted_attribute_value")
                        {
                            let raw_value = self.node_text(value_node, source);
                            let value = raw_value.trim_matches(|c| c == '"' || c == '\'');
                            let value_start_line = value_node.start_position().row as u32;
                            let value_start_col = value_node.start_position().column as u32 + 1;

                            // 共通パーサーを使用
                            let parsed_vars = parse_ng_repeat_expression(value);
                            let result: Vec<LocalVariableScope> = parsed_vars
                                .into_iter()
                                .map(|var| LocalVariableScope {
                                    name: var.name,
                                    source: var.source,
                                    uri: uri.clone(),
                                    scope_start_line,
                                    scope_end_line,
                                    name_start_line: value_start_line,
                                    name_start_col: value_start_col + var.offset as u32,
                                    name_end_line: value_start_line,
                                    name_end_col: value_start_col + var.offset as u32 + var.len as u32,
                                })
                                .collect();

                            if result.is_empty() {
                                return None;
                            }
                            return Some(result);
                        }
                    }
                }
            }
        }
        None
    }

    /// ng-initからローカル変数を抽出
    fn extract_local_vars_from_ng_init(
        &self,
        start_tag: Node,
        source: &str,
        uri: &Url,
        scope_start_line: u32,
        scope_end_line: u32,
    ) -> Option<Vec<LocalVariableScope>> {
        let mut cursor = start_tag.walk();
        for child in start_tag.children(&mut cursor) {
            if child.kind() == "attribute" {
                if let Some(name_node) = self.find_child_by_kind(child, "attribute_name") {
                    let attr_name = self.node_text(name_node, source);

                    if attr_name == "ng-init" || attr_name == "data-ng-init" {
                        if let Some(value_node) =
                            self.find_child_by_kind(child, "quoted_attribute_value")
                        {
                            let raw_value = self.node_text(value_node, source);
                            let value = raw_value.trim_matches(|c| c == '"' || c == '\'');
                            let value_start_line = value_node.start_position().row as u32;
                            let value_start_col = value_node.start_position().column as u32 + 1;

                            // 共通パーサーを使用
                            let parsed_vars = parse_ng_init_expression(value);
                            let result: Vec<LocalVariableScope> = parsed_vars
                                .into_iter()
                                .map(|var| LocalVariableScope {
                                    name: var.name,
                                    source: var.source,
                                    uri: uri.clone(),
                                    scope_start_line,
                                    scope_end_line,
                                    name_start_line: value_start_line,
                                    name_start_col: value_start_col + var.offset as u32,
                                    name_end_line: value_start_line,
                                    name_end_col: value_start_col + var.offset as u32 + var.len as u32,
                                })
                                .collect();

                            if result.is_empty() {
                                return None;
                            }
                            return Some(result);
                        }
                    }
                }
            }
        }
        None
    }

    /// <form name="x">からフォームバインディングを抽出
    /// 動的な name="{{...}}" はスキップする
    pub(super) fn extract_form_name_from_tag(
        &self,
        start_tag: Node,
        source: &str,
        uri: &Url,
        scope_start_line: u32,
        scope_end_line: u32,
    ) -> Option<FormBindingScope> {
        // タグ名を取得
        let tag_name_node = self.find_child_by_kind(start_tag, "tag_name")?;
        let tag_name = self.node_text(tag_name_node, source);

        // <form>タグのみ対象
        if tag_name != "form" {
            return None;
        }

        // name属性を探す
        let mut cursor = start_tag.walk();
        for child in start_tag.children(&mut cursor) {
            if child.kind() == "attribute" {
                if let Some(name_node) = self.find_child_by_kind(child, "attribute_name") {
                    let attr_name = self.node_text(name_node, source);

                    if attr_name == "name" {
                        if let Some(value_node) =
                            self.find_child_by_kind(child, "quoted_attribute_value")
                        {
                            let raw_value = self.node_text(value_node, source);
                            // クォートを除去
                            let value = raw_value.trim_matches(|c| c == '"' || c == '\'');

                            // 動的バインディング {{...}} をスキップ
                            if value.contains("{{") || value.contains("}}") {
                                return None;
                            }

                            // 空でない有効な識別子のみ
                            if value.is_empty() || !is_valid_identifier(value) {
                                return None;
                            }

                            // 値の位置を計算（クォートの次から）
                            let value_start_line = value_node.start_position().row as u32;
                            let value_start_col = value_node.start_position().column as u32 + 1; // クォート分

                            return Some(FormBindingScope {
                                name: value.to_string(),
                                uri: uri.clone(),
                                scope_start_line,
                                scope_end_line,
                                name_start_line: value_start_line,
                                name_start_col: value_start_col,
                                name_end_line: value_start_line,
                                name_end_col: value_start_col + value.len() as u32,
                                controller_depth: 0, // 呼び出し元で設定
                            });
                        }
                    }
                }
            }
        }
        None
    }
}
