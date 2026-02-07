//! ng-controllerスコープの収集

use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use crate::model::{HtmlControllerScope, Span, SymbolReference};

use super::HtmlAngularJsAnalyzer;

/// コントローラースコープ情報（収集時に使用）
#[derive(Clone, Debug)]
pub(super) struct ControllerScopeInfo {
    pub name: String,
    pub start_line: u32,
    pub end_line: u32,
}

impl HtmlAngularJsAnalyzer {
    /// ng-controllerスコープのみを収集（Pass 1用）
    /// ng-includeバインディングは収集しない
    pub(super) fn collect_controller_scopes_only_from_tree(
        &self,
        node: Node,
        source: &str,
        uri: &Url,
    ) {
        self.collect_controller_scopes_only_impl(node, source, uri);
    }

    /// ng-controllerスコープのみを収集（実装）
    fn collect_controller_scopes_only_impl(
        &self,
        node: Node,
        source: &str,
        uri: &Url,
    ) {
        if node.kind() == "element" {
            let scope_start_line = node.start_position().row as u32;
            let scope_end_line = node.end_position().row as u32;

            // 開始タグから属性を取得
            if let Some(start_tag) = self.find_child_by_kind(node, "start_tag") {
                // ng-controllerをチェック（位置情報付き）
                if let Some((controller_name, alias, name_start_line, name_start_col, name_end_line, name_end_col)) =
                    self.get_ng_controller_attribute_with_position(start_tag, source)
                {
                    // ng-controllerスコープを登録
                    let scope = HtmlControllerScope {
                        controller_name: controller_name.clone(),
                        alias,
                        uri: uri.clone(),
                        start_line: scope_start_line,
                        end_line: scope_end_line,
                    };
                    self.index.controllers.add_html_controller_scope(scope);

                    // コントローラー名への参照を登録（定義ジャンプ用）
                    let reference = SymbolReference {
                        name: controller_name,
                        uri: uri.clone(),
                        span: Span::new(
                            name_start_line,
                            name_start_col,
                            name_end_line,
                            name_end_col,
                        ),
                    };
                    self.index.definitions.add_reference(reference);
                }
            }

            // 子要素を再帰的に処理
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.collect_controller_scopes_only_impl(child, source, uri);
            }
        } else {
            // 子ノードを再帰的に処理
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.collect_controller_scopes_only_impl(child, source, uri);
            }
        }
    }

    /// ng-controller属性の値を取得
    /// 戻り値: (コントローラー名, alias)
    /// 例: "UserController as vm" -> ("UserController", Some("vm"))
    pub(super) fn get_ng_controller_attribute(&self, start_tag: Node, source: &str) -> Option<(String, Option<String>)> {
        self.get_ng_controller_attribute_with_position(start_tag, source)
            .map(|(name, alias, _, _, _, _)| (name, alias))
    }

    /// ng-controller属性の値と位置を取得
    /// 戻り値: (コントローラー名, alias, start_line, start_col, end_line, end_col)
    /// 位置はコントローラー名の位置（クォート除く）
    pub(super) fn get_ng_controller_attribute_with_position(
        &self,
        start_tag: Node,
        source: &str,
    ) -> Option<(String, Option<String>, u32, u32, u32, u32)> {
        let mut cursor = start_tag.walk();
        for child in start_tag.children(&mut cursor) {
            if child.kind() == "attribute" {
                if let Some(name) = self.find_child_by_kind(child, "attribute_name") {
                    let attr_name = self.node_text(name, source);
                    if attr_name == "ng-controller" || attr_name == "data-ng-controller" {
                        if let Some(value_node) = self.find_child_by_kind(child, "quoted_attribute_value") {
                            let raw_value = self.node_text(value_node, source);
                            // クォートを除去
                            let value = raw_value.trim_matches(|c| c == '"' || c == '\'');
                            // "Controller as alias"形式をパース
                            let parts: Vec<&str> = value.split_whitespace().collect();
                            let controller_name = parts.first().unwrap_or(&value).to_string();
                            let alias = if parts.len() >= 3 && parts[1].eq_ignore_ascii_case("as") {
                                Some(parts[2].to_string())
                            } else {
                                None
                            };

                            // コントローラー名の位置を計算（クォートの後から）
                            let start_line = value_node.start_position().row as u32;
                            let start_col = value_node.start_position().column as u32 + 1; // クォート分
                            let end_line = start_line;
                            let end_col = start_col + controller_name.len() as u32;

                            return Some((controller_name, alias, start_line, start_col, end_line, end_col));
                        }
                    }
                }
            }
        }
        None
    }

    /// ng-include属性またはsrc属性（<ng-include>要素用）の値を取得
    pub(super) fn get_ng_include_attribute(&self, start_tag: Node, source: &str) -> Option<String> {
        // タグ名を取得
        let tag_name_node = self.find_child_by_kind(start_tag, "tag_name");
        let tag_name = tag_name_node.map(|n| self.node_text(n, source));

        let mut cursor = start_tag.walk();
        for child in start_tag.children(&mut cursor) {
            if child.kind() == "attribute" {
                if let Some(name) = self.find_child_by_kind(child, "attribute_name") {
                    let attr_name = self.node_text(name, source);

                    // ng-include属性または<ng-include>要素のsrc属性をチェック
                    let is_ng_include = attr_name == "ng-include" || attr_name == "data-ng-include";
                    let is_ng_include_src = (tag_name.as_deref() == Some("ng-include") || tag_name.as_deref() == Some("data-ng-include"))
                        && attr_name == "src";

                    if is_ng_include || is_ng_include_src {
                        if let Some(value) = self.find_child_by_kind(child, "quoted_attribute_value") {
                            let raw_value = self.node_text(value, source);
                            // 外側のクォート（最初と最後の1文字）を除去
                            let value = if raw_value.len() >= 2 {
                                &raw_value[1..raw_value.len() - 1]
                            } else {
                                raw_value.as_str()
                            };
                            // 文字列リテラル部分を抽出
                            if let Some(template_path) = self.extract_string_literal(value) {
                                return Some(template_path);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// 式から最初の文字列リテラルを抽出
    /// 例: "'template.html'" -> "template.html"
    /// 例: "'path/to/file.html?' + version" -> "path/to/file.html?"
    pub(super) fn extract_string_literal(&self, expr: &str) -> Option<String> {
        let expr = expr.trim();

        // シングルクォートで始まる場合
        if expr.starts_with('\'') {
            if let Some(end_idx) = expr[1..].find('\'') {
                return Some(expr[1..end_idx + 1].to_string());
            }
        }

        // ダブルクォートで始まる場合
        if expr.starts_with('"') {
            if let Some(end_idx) = expr[1..].find('"') {
                return Some(expr[1..end_idx + 1].to_string());
            }
        }

        None
    }

    /// ng-view要素かどうかを判定
    /// 対象パターン:
    /// - `<ng-view>` / `<data-ng-view>` タグ
    /// - `ng-view` / `data-ng-view` 属性を持つ要素
    pub(super) fn is_ng_view_element(&self, start_tag: Node, source: &str) -> bool {
        // タグ名をチェック（<ng-view>または<data-ng-view>）
        if let Some(tag_name_node) = self.find_child_by_kind(start_tag, "tag_name") {
            let tag_name = self.node_text(tag_name_node, source);
            if tag_name == "ng-view" || tag_name == "data-ng-view" {
                return true;
            }
        }

        // 属性をチェック（ng-viewまたはdata-ng-view属性）
        let mut cursor = start_tag.walk();
        for child in start_tag.children(&mut cursor) {
            if child.kind() == "attribute" {
                if let Some(name_node) = self.find_child_by_kind(child, "attribute_name") {
                    let attr_name = self.node_text(name_node, source);
                    if attr_name == "ng-view" || attr_name == "data-ng-view" {
                        return true;
                    }
                }
            }
        }

        false
    }
}
