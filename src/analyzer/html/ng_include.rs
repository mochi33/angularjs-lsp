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
                            // UTF-16 column 化 (tree-sitter の column は byte)
                            let value_start_byte_col = value_node.start_position().column + 1;
                            let value_start_col = self.byte_col_to_utf16_col(
                                source,
                                value_start_line as usize,
                                value_start_byte_col,
                            );

                            // 共通パーサーを使用
                            let parsed_vars = parse_ng_repeat_expression(value);
                            let result: Vec<LocalVariableScope> = parsed_vars
                                .into_iter()
                                .map(|var| {
                                    // var.offset / var.len は属性値内のバイト単位なので
                                    // UTF-16 単位に変換する
                                    let utf16_offset = self
                                        .byte_offset_to_utf16_offset(value, var.offset);
                                    let var_text = &value[var.offset..var.offset + var.len];
                                    let utf16_len: usize = var_text
                                        .chars()
                                        .map(|c| c.len_utf16())
                                        .sum();
                                    LocalVariableScope {
                                        name: var.name,
                                        source: var.source,
                                        uri: uri.clone(),
                                        scope_start_line,
                                        scope_end_line,
                                        name_start_line: value_start_line,
                                        name_start_col: value_start_col + utf16_offset as u32,
                                        name_end_line: value_start_line,
                                        name_end_col: value_start_col
                                            + utf16_offset as u32
                                            + utf16_len as u32,
                                    }
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
                            // UTF-16 column 化 (tree-sitter の column は byte)
                            let value_start_byte_col = value_node.start_position().column + 1;
                            let value_start_col = self.byte_col_to_utf16_col(
                                source,
                                value_start_line as usize,
                                value_start_byte_col,
                            );

                            // 共通パーサーを使用
                            let parsed_vars = parse_ng_init_expression(value);
                            let result: Vec<LocalVariableScope> = parsed_vars
                                .into_iter()
                                .map(|var| {
                                    let utf16_offset = self
                                        .byte_offset_to_utf16_offset(value, var.offset);
                                    let var_text = &value[var.offset..var.offset + var.len];
                                    let utf16_len: usize = var_text
                                        .chars()
                                        .map(|c| c.len_utf16())
                                        .sum();
                                    LocalVariableScope {
                                        name: var.name,
                                        source: var.source,
                                        uri: uri.clone(),
                                        scope_start_line,
                                        scope_end_line,
                                        name_start_line: value_start_line,
                                        name_start_col: value_start_col + utf16_offset as u32,
                                        name_end_line: value_start_line,
                                        name_end_col: value_start_col
                                            + utf16_offset as u32
                                            + utf16_len as u32,
                                    }
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

                            // 値の位置を計算（クォートの次から、UTF-16 単位）
                            let value_start_line = value_node.start_position().row as u32;
                            let value_start_byte_col = value_node.start_position().column + 1;
                            let value_start_col = self.byte_col_to_utf16_col(
                                source,
                                value_start_line as usize,
                                value_start_byte_col,
                            );
                            let value_utf16_len: usize =
                                value.chars().map(|c| c.len_utf16()).sum();

                            return Some(FormBindingScope {
                                name: value.to_string(),
                                uri: uri.clone(),
                                scope_start_line,
                                scope_end_line,
                                name_start_line: value_start_line,
                                name_start_col: value_start_col,
                                name_end_line: value_start_line,
                                name_end_col: value_start_col + value_utf16_len as u32,
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tower_lsp::lsp_types::Url;

    use crate::analyzer::html::HtmlAngularJsAnalyzer;
    use crate::analyzer::js::AngularJsAnalyzer;
    use crate::index::Index;

    fn analyze(source: &str) -> (Arc<Index>, Url) {
        let index = Arc::new(Index::new());
        let js = Arc::new(AngularJsAnalyzer::new(Arc::clone(&index)));
        let html = HtmlAngularJsAnalyzer::new(Arc::clone(&index), js);
        let uri = Url::parse("file:///test.html").unwrap();
        html.analyze_document(&uri, source);
        (index, uri)
    }

    #[test]
    fn form_binding_position_is_utf16_when_line_has_japanese_before() {
        // tree-sitter の column は UTF-8 byte なので、同一行に多バイト文字 (日本語等)
        // が含まれていると UTF-16 変換しないと LSP 側で列がずれる。
        // 例: <span title="日本語">の右に <form name="myForm"> がある場合、
        // 日本語 3 文字 (UTF-8: 9 byte / UTF-16: 3 unit) なので byte と utf-16 の差は 6。
        let source =
            r#"<div><span title="日本語"></span><form name="myForm"></form></div>"#;
        let (index, uri) = analyze(source);

        let bindings = index.html.get_all_form_bindings(&uri);
        assert_eq!(bindings.len(), 1, "form name should be registered");
        let b = &bindings[0];
        assert_eq!(b.name, "myForm");

        // 期待される UTF-16 column を実測 (テスト fixture が変わったときの追従用)
        let line = source;
        let needle = "myForm";
        let prefix_byte = line.find(needle).unwrap();
        let utf16_start: u32 = line[..prefix_byte]
            .chars()
            .map(|c| c.len_utf16() as u32)
            .sum();
        let utf16_end = utf16_start + needle.chars().map(|c| c.len_utf16() as u32).sum::<u32>();

        assert_eq!(
            b.name_start_col, utf16_start,
            "form name の start_col は UTF-16 単位であるべき"
        );
        assert_eq!(
            b.name_end_col, utf16_end,
            "form name の end_col は UTF-16 単位であるべき"
        );
    }

    #[test]
    fn ng_repeat_inherited_local_variable_position_is_utf16_when_line_has_japanese() {
        // <div ng-include> 経由で継承する `LocalVariableScope` の name_start_col も
        // UTF-16 単位でなければ、子テンプレートでの参照解決位置がずれる。
        let source = r#"<div title="日本語コメント" ng-repeat="item in items" ng-include="'child.html'"></div>"#;
        let (index, _uri) = analyze(source);

        let parent_uri = Url::parse("file:///test.html").unwrap();
        // 子側 (child.html) から継承を逆引き
        let child_uri = Url::parse("file:///child.html").unwrap();
        let inherited = index
            .templates
            .get_inherited_local_variables_for_template(&child_uri);
        let item = inherited.iter().find(|v| v.name == "item");
        assert!(item.is_some(), "ng-repeat の item 変数が継承されているはず");
        let item = item.unwrap();

        // 期待値: 親 source 上の "item" の utf-16 col
        let prefix_byte = source.find("item in items").unwrap();
        let utf16_start: u32 = source[..prefix_byte]
            .chars()
            .map(|c| c.len_utf16() as u32)
            .sum();

        assert_eq!(
            item.name_start_col, utf16_start,
            "継承された ng-repeat 変数の start_col は UTF-16 単位であるべき"
        );

        // parent_uri が使われない警告対策のためダミー利用
        let _ = parent_uri;
    }

    /// JS analyzer も合わせて回す版。実環境同様 `groupSelector` を controller alias
    /// として認識させた上で HTML を解析する。
    fn analyze_with_js(html_source: &str, js_source: &str) -> (Arc<Index>, Url, Url) {
        let index = Arc::new(Index::new());
        let js_analyzer = Arc::new(AngularJsAnalyzer::new(Arc::clone(&index)));
        let html = HtmlAngularJsAnalyzer::new(Arc::clone(&index), Arc::clone(&js_analyzer));
        let html_uri = Url::parse("file:///test.html").unwrap();
        let js_uri = Url::parse("file:///test.js").unwrap();

        // JS 先行解析 (alias 解決のため)
        js_analyzer.analyze_document(&js_uri, js_source);
        html.analyze_document(&html_uri, html_source);
        (index, html_uri, js_uri)
    }

    #[test]
    fn alias_dot_property_ref_covers_only_property_span() {
        // 以前は `alias.property` 形式の参照が **alias + dot + property 全体** を
        // 覆う position で登録されていた。dedup の overlap ルール (長い token 優先) で
        // alias 単独 ref が消され、結果として alias 部分まで METHOD 色で塗られていた
        // (ユーザーから見ると「token がずれている」状態)。
        //
        // 修正後は `alias.property` ref の span は **property 部分のみ** を覆い、
        // `alias` 単独 ref と被らないので両方が semantic token として正しく描画される。
        let html = "<div ng-click=\"groupSelector.showSelectGroupDialog()\"></div>\n";
        let js = "angular.module('app', []).component('groupSelector', {\n  templateUrl: 'test.html',\n  controller: function() { var vm = this; vm.showSelectGroupDialog = function() {}; },\n  controllerAs: 'groupSelector',\n});\n";
        let (index, uri, _) = analyze_with_js(html, js);

        let refs = index.html.get_html_scope_references(&uri);
        let lines: Vec<&str> = html.lines().collect();

        let alias_ref = refs
            .iter()
            .find(|r| r.property_path == "groupSelector")
            .expect("alias 単独 ref が存在するはず");
        let combined_ref = refs
            .iter()
            .find(|r| r.property_path == "groupSelector.showSelectGroupDialog")
            .expect("alias.property ref が存在するはず");

        let span_text = |r: &crate::model::HtmlScopeReference| -> String {
            let line = lines[r.start_line as usize];
            let line_utf16: Vec<u16> = line.encode_utf16().collect();
            String::from_utf16_lossy(&line_utf16[r.start_col as usize..r.end_col as usize])
        };

        assert_eq!(span_text(alias_ref), "groupSelector");
        // alias.property ref の span は **property のみ** を指していること
        assert_eq!(
            span_text(combined_ref),
            "showSelectGroupDialog",
            "alias.property ref の span は property のみを覆うべき"
        );

        // 2つの ref は overlap しないこと (encode_tokens dedup で消されない)
        assert!(
            alias_ref.end_col <= combined_ref.start_col
                || combined_ref.end_col <= alias_ref.start_col,
            "alias と alias.property の span は overlap してはいけない"
        );
    }

    #[test]
    fn ng_controller_reference_position_is_utf16_when_line_has_japanese() {
        // ng-controller のシンボル参照位置も UTF-16 化していないと、
        // find references / goto definition で controller 名のスパンがずれる。
        let source = r#"<div title="日本語の説明" ng-controller="MyCtrl"></div>"#;
        let (index, uri) = analyze(source);

        let refs = index.definitions.get_references("MyCtrl");
        let r = refs
            .iter()
            .find(|r| r.uri == uri)
            .expect("ng-controller は SymbolReference として登録される");

        let prefix_byte = source.find("MyCtrl").unwrap();
        let utf16_start: u32 = source[..prefix_byte]
            .chars()
            .map(|c| c.len_utf16() as u32)
            .sum();
        let utf16_end = utf16_start + "MyCtrl".len() as u32; // ASCII なので byte == utf16

        assert_eq!(r.span.start_col, utf16_start);
        assert_eq!(r.span.end_col, utf16_end);
    }
}
