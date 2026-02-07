mod component;
mod context;
mod di;
mod export;
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
            // 事前収集フェーズ:
            // 1. $inject パターン用の関数宣言とclass宣言を収集
            self.collect_function_declarations_for_inject(tree.root_node(), source, &mut ctx);
            // 2. $inject パターンを収集
            self.collect_inject_patterns(tree.root_node(), source, uri, &mut ctx);
            // 3. 関数/class参照パターンのコンポーネント登録を収集（$inject なしでも $scope 追跡可能に）
            self.collect_component_ref_scopes(tree.root_node(), source, uri, &mut ctx);
            // 本解析
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
    /// - `import_statement`: ES6 import文
    fn visit_node(&self, node: Node, source: &str, uri: &Url, ctx: &mut AnalyzerContext) {
        match node.kind() {
            "call_expression" => {
                self.analyze_call_expression(node, source, uri, ctx);
                self.analyze_method_call(node, source, uri, ctx);
            }
            "member_expression" => {
                self.analyze_member_access(node, source, uri, ctx);
                self.analyze_scope_member_access(node, source, uri, ctx);
                self.analyze_root_scope_member_access(node, source, uri, ctx);
            }
            "expression_statement" => {
                self.analyze_inject_pattern(node, source, uri);
            }
            "assignment_expression" => {
                self.analyze_scope_assignment(node, source, uri, ctx);
                self.analyze_root_scope_assignment(node, source, uri, ctx);
            }
            "identifier" => {
                self.analyze_identifier(node, source, uri, ctx);
            }
            "export_statement" => {
                self.analyze_export_statement(node, source, uri, ctx);
            }
            "import_statement" => {
                self.analyze_import_statement(node, source, uri);
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit_node(child, source, uri, ctx);
        }
    }

    /// ES6 import文を解析する
    ///
    /// 認識パターン:
    /// ```javascript
    /// import UserDetails from 'src/users/components/details/UserDetails';
    /// import { something } from 'path';  // named imports (現在は対象外)
    /// ```
    fn analyze_import_statement(&self, node: Node, source: &str, uri: &Url) {
        // デフォルトインポートの識別子を探す
        // import UserDetails from '...'
        //        ^^^^^^^^^^^ -> identifier inside import_clause
        let mut import_name: Option<String> = None;
        let mut import_path: Option<String> = None;

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "import_clause" => {
                    // import_clause 内の identifier を探す（デフォルトインポート）
                    let mut clause_cursor = child.walk();
                    for clause_child in child.children(&mut clause_cursor) {
                        if clause_child.kind() == "identifier" {
                            import_name = Some(self.node_text(clause_child, source));
                            break;
                        }
                    }
                }
                "string" => {
                    // import元のパス
                    import_path = Some(self.extract_string_value(child, source));
                }
                _ => {}
            }
        }

        // デフォルトインポートとパスの両方が見つかった場合、マッピングを登録
        if let (Some(name), Some(path)) = (import_name, import_path) {
            self.index.add_import(uri, name, path);
        }
    }
}
