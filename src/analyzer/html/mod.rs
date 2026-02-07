//! HTML内のAngularJSディレクティブを解析するモジュール

use std::sync::{Arc, RwLock};

use tower_lsp::lsp_types::Url;
use tree_sitter::{Node, Tree};

use crate::config::InterpolateConfig;
use crate::index::Index;

pub mod controller;
pub mod directive_reference;
pub mod directives;
pub mod expression;
pub mod form;
pub mod local_variable;
pub mod ng_include;
pub mod parser;
pub mod scope_reference;
pub mod script;
pub mod variable_parser;

use controller::ControllerScopeInfo;
pub use script::EmbeddedScript;

/// HTML内のAngularJSディレクティブを解析するアナライザー
pub struct HtmlAngularJsAnalyzer {
    index: Arc<Index>,
    js_analyzer: Arc<crate::analyzer::js::AngularJsAnalyzer>,
    interpolate: RwLock<InterpolateConfig>,
}

impl HtmlAngularJsAnalyzer {
    pub fn new(index: Arc<Index>, js_analyzer: Arc<crate::analyzer::js::AngularJsAnalyzer>) -> Self {
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

    /// HTMLドキュメントを解析（単独ファイル解析用）
    /// 全パス（Pass 1, 1.5, 2, 3）を実行
    /// テストや単一ファイル更新時に使用
    pub fn analyze_document(&self, uri: &Url, source: &str) {
        let mut html_parser = parser::HtmlParser::new();
        if let Some(tree) = html_parser.parse(source) {
            self.analyze_document_with_tree(uri, source, &tree);
        }
    }

    /// HTMLドキュメントを解析し、埋め込みスクリプトも抽出
    /// on_change/on_openで使用（単一パースで両方の処理を実行）
    pub fn analyze_document_and_extract_scripts(&self, uri: &Url, source: &str) -> Vec<EmbeddedScript> {
        let mut html_parser = parser::HtmlParser::new();
        if let Some(tree) = html_parser.parse(source) {
            self.analyze_document_with_tree(uri, source, &tree);
            Self::extract_scripts_from_tree(tree.root_node(), source)
        } else {
            Vec::new()
        }
    }

    /// 事前にパースしたTreeでHTMLドキュメントを解析
    fn analyze_document_with_tree(&self, uri: &Url, source: &str, tree: &Tree) {
        // 既存情報をクリア
        self.index.clear_document(uri);

        // Pass 1: ng-controllerスコープ収集
        self.collect_controller_scopes_only_with_tree(uri, source, tree);

        // Pass 1.5: ng-includeバインディング収集
        self.collect_ng_include_bindings_with_tree(uri, source, tree);

        // Pass 2: フォームバインディング収集
        self.collect_form_bindings_only_with_tree(uri, source, tree);

        // Pass 3: 参照収集
        self.analyze_document_references_only_with_tree(uri, source, tree);
    }

    /// HTMLドキュメントの参照のみを解析（Pass 3用）
    /// ng-controllerスコープ、ng-includeバインディング、フォームバインディングは
    /// 事前のPass 1, 1.5, 2で収集済みであることを前提とする
    /// scan_workspaceから使用
    pub fn analyze_document_references_only(&self, uri: &Url, source: &str) {
        let mut html_parser = parser::HtmlParser::new();
        if let Some(tree) = html_parser.parse(source) {
            self.analyze_document_references_only_with_tree(uri, source, &tree);
        }
    }

    /// HTMLドキュメントの参照のみを解析（Pass 3用、Tree再利用版）
    pub fn analyze_document_references_only_with_tree(&self, uri: &Url, source: &str, tree: &Tree) {
        // Pass 3で収集する情報のみクリア（Pass 1, 1.5, 2の情報は保持）
        self.index.clear_html_references(uri);

        // ローカル変数定義を収集（ng-init, ng-repeat由来）
        // これをスコープ参照収集より先に行うことで、ローカル変数をフィルタリングできる
        self.collect_local_variable_definitions(tree.root_node(), source, uri);

        // $scope参照を収集（ローカル変数はフィルタリング）
        self.collect_scope_references(tree.root_node(), source, uri);

        // カスタムディレクティブ参照を収集
        self.collect_directive_references(tree.root_node(), source, uri);

        // ローカル変数参照を収集
        let mut active_scopes = std::collections::HashMap::new();

        // 継承されたローカル変数も active_scopes に追加（ng-include経由）
        // 子テンプレート全体で有効なので、scope範囲をファイル全体に設定
        let inherited_vars = self.index.templates.get_inherited_local_variables_for_template(uri);
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

    /// ng-controllerスコープのみを収集（Pass 1用）
    /// 全HTMLファイルのng-controllerスコープを先に登録する
    pub fn collect_controller_scopes_only(&self, uri: &Url, source: &str) {
        let mut html_parser = parser::HtmlParser::new();
        if let Some(tree) = html_parser.parse(source) {
            self.collect_controller_scopes_only_with_tree(uri, source, &tree);
        }
    }

    /// ng-controllerスコープのみを収集（Pass 1用、Tree再利用版）
    pub fn collect_controller_scopes_only_with_tree(&self, uri: &Url, source: &str, tree: &Tree) {
        // このHTMLファイルを解析済みとしてマーク
        self.index.mark_html_analyzed(uri);
        // ng-controllerスコープのみを収集
        self.collect_controller_scopes_only_from_tree(tree.root_node(), source, uri);
    }

    /// ng-includeバインディングを収集（Pass 1.5用）
    /// ng-controllerスコープが全て確定した後に呼び出される
    /// 継承チェーンを考慮してコントローラー情報を継承
    pub fn collect_ng_include_bindings(&self, uri: &Url, source: &str) {
        let mut html_parser = parser::HtmlParser::new();
        if let Some(tree) = html_parser.parse(source) {
            self.collect_ng_include_bindings_with_tree(uri, source, &tree);
        }
    }

    /// ng-includeバインディングを収集（Pass 1.5用、Tree再利用版）
    pub fn collect_ng_include_bindings_with_tree(&self, uri: &Url, source: &str, tree: &Tree) {
        // 初期スタックを構築
        let mut controller_stack: Vec<ControllerScopeInfo> = Vec::new();

        // ng-includeで継承されたコントローラーを追加（親HTMLから）
        let inherited = self.index.templates.get_inherited_controllers_for_template(uri);
        for name in inherited {
            controller_stack.push(ControllerScopeInfo {
                name,
                start_line: 0,
                end_line: u32::MAX,
            });
        }

        // ng-viewからの継承を追加（$routeProviderテンプレートの場合）
        // 新アーキテクチャではng_view_bindingsとng_include_bindingsが別DashMapのため
        // ロック競合は発生しない
        let ng_view_inherited = self.index.templates.get_ng_view_inherited_controllers(uri);
        for name in ng_view_inherited {
            if !controller_stack.iter().any(|c| c.name == name) {
                controller_stack.push(ControllerScopeInfo {
                    name,
                    start_line: 0,
                    end_line: u32::MAX,
                });
            }
        }

        // テンプレートバインディング経由のコントローラーを追加
        if let Some(controller) = self.index.templates.get_controller_for_template(uri) {
            if !controller_stack.iter().any(|c| c.name == controller) {
                controller_stack.push(ControllerScopeInfo {
                    name: controller,
                    start_line: 0,
                    end_line: u32::MAX,
                });
            }
        }

        self.collect_ng_include_bindings_from_tree(tree.root_node(), source, uri, &mut controller_stack);
    }

    /// フォームバインディングのみを収集（Pass 2用）
    /// ng-include関係が確定した後に呼び出される
    /// これにより、子HTMLでも親のフォームバインディングを参照可能になる
    pub fn collect_form_bindings_only(&self, uri: &Url, source: &str) {
        let mut html_parser = parser::HtmlParser::new();
        if let Some(tree) = html_parser.parse(source) {
            self.collect_form_bindings_only_with_tree(uri, source, &tree);
        }
    }

    /// フォームバインディングのみを収集（Pass 2用、Tree再利用版）
    pub fn collect_form_bindings_only_with_tree(&self, uri: &Url, source: &str, tree: &Tree) {
        // ng-includeで継承されたコントローラーを初期スタックに追加
        let mut controller_stack: Vec<ControllerScopeInfo> = Vec::new();

        let inherited = self.index.templates.get_inherited_controllers_for_template(uri);
        for name in inherited {
            controller_stack.push(ControllerScopeInfo {
                name,
                start_line: 0,
                end_line: u32::MAX,
            });
        }

        // テンプレートバインディング経由のコントローラーを追加
        if let Some(controller) = self.index.templates.get_controller_for_template(uri) {
            if !controller_stack.iter().any(|c| c.name == controller) {
                controller_stack.push(ControllerScopeInfo {
                    name: controller,
                    start_line: 0,
                    end_line: u32::MAX,
                });
            }
        }

        self.collect_form_bindings_from_tree(tree.root_node(), source, uri, &mut controller_stack);
    }
}
