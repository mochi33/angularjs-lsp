mod component;
mod context;
mod di;
mod reference;
mod scope;
mod service_method;
mod utils;

#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use tower_lsp::lsp_types::Url;
use tree_sitter::{Node, Tree};

use super::JsParser;
use crate::index::SymbolIndex;
use context::AnalyzerContext;

/// AngularJS 1.x のコードを解析し、シンボル定義と参照を抽出するアナライザー
pub struct AngularJsAnalyzer {
    pub(crate) index: Arc<SymbolIndex>,
    /// 行番号オフセット（HTML内のscriptタグ用）
    pub(crate) line_offset: AtomicU32,
}

impl AngularJsAnalyzer {
    pub fn new(index: Arc<SymbolIndex>) -> Self {
        Self {
            index,
            line_offset: AtomicU32::new(0),
        }
    }

    /// ドキュメントを解析してシンボルをインデックスに追加する
    ///
    /// 既存のドキュメント情報をクリアしてから解析を行う
    pub fn analyze_document(&self, uri: &Url, source: &str) {
        self.analyze_document_with_options(uri, source, true);
    }

    /// ドキュメントを解析してシンボルをインデックスに追加する
    ///
    /// # Arguments
    /// * `uri` - ドキュメントのURI
    /// * `source` - ソースコード
    /// * `clear` - true: 既存情報をクリア, false: 追記モード（2パス目用）
    pub fn analyze_document_with_options(&self, uri: &Url, source: &str, clear: bool) {
        self.line_offset.store(0, Ordering::Relaxed);
        self.analyze_internal(uri, source, clear);
    }

    /// HTML内のscriptタグなど、埋め込みJSを解析する
    ///
    /// # Arguments
    /// * `uri` - ドキュメントのURI
    /// * `source` - ソースコード（scriptタグの中身）
    /// * `line_offset` - 行番号オフセット（scriptタグの開始行）
    pub fn analyze_embedded_script(&self, uri: &Url, source: &str, line_offset: u32) {
        self.line_offset.store(line_offset, Ordering::Relaxed);
        self.analyze_internal(uri, source, false);
        self.line_offset.store(0, Ordering::Relaxed);
    }

    fn analyze_internal(&self, uri: &Url, source: &str, clear: bool) {
        let mut parser = JsParser::new();

        if let Some(tree) = parser.parse(source) {
            if clear {
                self.index.clear_document(uri);
            }
            let mut ctx = AnalyzerContext::new();
            // 事前収集: $inject パターン用の関数宣言と $inject パターンを収集
            self.collect_function_declarations_for_inject(tree.root_node(), source, &mut ctx);
            self.collect_inject_patterns(tree.root_node(), source, uri, &mut ctx);
            self.traverse_tree(&tree, source, uri, &mut ctx);
        }
    }

    /// AST全体を走査する
    fn traverse_tree(&self, tree: &Tree, source: &str, uri: &Url, ctx: &mut AnalyzerContext) {
        let root = tree.root_node();
        self.visit_node(root, source, uri, ctx);
    }

    /// ASTノードを訪問し、種類に応じた解析を行う
    ///
    /// 認識するノード:
    /// - `call_expression`: 関数呼び出し（angular.module(), .controller() 等）
    /// - `member_expression`: プロパティアクセス（Service.method, $scope.property）
    /// - `expression_statement`: 式文（$inject パターン）
    /// - `assignment_expression`: 代入式（$scope.property = value）
    /// - `identifier`: 識別子（サービス名等の参照）
    fn visit_node(&self, node: Node, source: &str, uri: &Url, ctx: &mut AnalyzerContext) {
        match node.kind() {
            "call_expression" => {
                self.analyze_call_expression(node, source, uri, ctx);
                self.analyze_method_call(node, source, uri, ctx);
            }
            "member_expression" => {
                self.analyze_member_access(node, source, uri, ctx);
                self.analyze_scope_member_access(node, source, uri, ctx);
            }
            "expression_statement" => {
                self.analyze_inject_pattern(node, source, uri);
            }
            "assignment_expression" => {
                self.analyze_scope_assignment(node, source, uri, ctx);
            }
            "identifier" => {
                self.analyze_identifier(node, source, uri, ctx);
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit_node(child, source, uri, ctx);
        }
    }
}
