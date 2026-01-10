//! HTML内のAngularJSディレクティブを解析するモジュール

use std::sync::{Arc, RwLock};

use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::{AngularJsAnalyzer, HtmlParser, JsParser};
use crate::config::InterpolateConfig;
use crate::index::SymbolIndex;

mod controller;
mod directives;
mod expression;
mod local_variable;
mod scope_reference;
mod script;

#[cfg(test)]
mod tests;

/// HTML内のAngularJSディレクティブを解析するアナライザー
pub struct HtmlAngularJsAnalyzer {
    index: Arc<SymbolIndex>,
    js_analyzer: Arc<AngularJsAnalyzer>,
    interpolate: RwLock<InterpolateConfig>,
}

impl HtmlAngularJsAnalyzer {
    pub fn new(index: Arc<SymbolIndex>, js_analyzer: Arc<AngularJsAnalyzer>) -> Self {
        Self {
            index,
            js_analyzer,
            interpolate: RwLock::new(InterpolateConfig::default()),
        }
    }

    /// interpolate設定を更新
    pub fn set_interpolate_config(&self, config: InterpolateConfig) {
        if let Ok(mut interpolate) = self.interpolate.write() {
            *interpolate = config;
        }
    }

    /// 現在のinterpolate設定を取得
    pub(self) fn get_interpolate_symbols(&self) -> (String, String) {
        if let Ok(config) = self.interpolate.read() {
            (config.start_symbol.clone(), config.end_symbol.clone())
        } else {
            ("{{".to_string(), "}}".to_string())
        }
    }

    /// HTMLドキュメントを解析
    pub fn analyze_document(&self, uri: &Url, source: &str) {
        let mut parser = HtmlParser::new();

        if let Some(tree) = parser.parse(source) {
            // 既存のHTML情報をクリア
            self.index.clear_document(uri);

            // Pass 1: ng-controllerスコープとng-includeを収集
            // 初期スタックには、このHTMLにバインドされているコントローラーを含める
            // 1. ng-includeで継承されたコントローラー（親HTMLから）
            // 2. テンプレートバインディング経由のコントローラー（$routeProvider, $uibModal）
            let mut controller_stack: Vec<String> = Vec::new();

            // ng-includeで継承されたコントローラーを追加
            let inherited = self.index.get_inherited_controllers_for_template(uri);
            controller_stack.extend(inherited);

            // テンプレートバインディング経由のコントローラーを追加
            if let Some(controller) = self.index.get_controller_for_template(uri) {
                if !controller_stack.contains(&controller) {
                    controller_stack.push(controller);
                }
            }

            self.collect_controller_scopes_and_includes(tree.root_node(), source, uri, &mut controller_stack);

            // Pass 2: <script>タグ内のJSを解析してバインディングを抽出
            self.analyze_script_tags(tree.root_node(), source, uri);

            // Pass 3: ローカル変数定義を収集（ng-init, ng-repeat由来）
            // これをスコープ参照収集より先に行うことで、ローカル変数をフィルタリングできる
            self.collect_local_variable_definitions(tree.root_node(), source, uri);

            // Pass 4: $scope参照を収集（ローカル変数はフィルタリング）
            self.collect_scope_references(tree.root_node(), source, uri);

            // Pass 5: ローカル変数参照を収集
            let mut active_scopes = std::collections::HashMap::new();

            // 継承されたローカル変数も active_scopes に追加（ng-include経由）
            // 子テンプレート全体で有効なので、scope範囲をファイル全体に設定
            let inherited_vars = self.index.get_inherited_local_variables_for_template(uri);
            for var in inherited_vars {
                // 継承された変数は子テンプレート全体で有効
                active_scopes.insert(var.name.clone(), (0, u32::MAX));
            }

            self.collect_local_variable_references(
                tree.root_node(),
                source,
                uri,
                &mut active_scopes,
            );
        }
    }

    /// 指定した種類の子ノードを検索
    pub(self) fn find_child_by_kind<'a>(&self, node: Node<'a>, kind: &str) -> Option<Node<'a>> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == kind {
                return Some(child);
            }
        }
        None
    }

    /// ノードのテキストを取得
    pub(self) fn node_text(&self, node: Node, source: &str) -> String {
        source[node.byte_range()].to_string()
    }

    /// 文字列ノードから値を取得（クォートを除去）
    pub(self) fn extract_string_value(&self, node: Node, source: &str) -> String {
        let text = self.node_text(node, source);
        text.trim_matches(|c| c == '"' || c == '\'').to_string()
    }
}