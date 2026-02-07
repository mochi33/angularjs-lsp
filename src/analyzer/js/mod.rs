mod component;
mod context;
mod di;
mod export;
mod module_chain;
mod parser;
mod reference;
mod scope;
mod service_method;

#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use tower_lsp::lsp_types::Url;
use tree_sitter::{Node, Tree};

use crate::index::Index;
use context::AnalyzerContext;
use parser::JsParser;

/// AngularJS 1.x のコードを解析し、シンボル定義と参照を抽出するアナライザー
pub struct AngularJsAnalyzer {
    pub(crate) index: Arc<Index>,
    /// 行番号オフセット（HTML内のscriptタグ用）
    pub(crate) line_offset: AtomicU32,
}

impl AngularJsAnalyzer {
    pub fn new(index: Arc<Index>) -> Self {
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
            self.index.exports.add_import(uri, name, path);
        }
    }

    // ========== Utility methods ==========

    /// 行番号にオフセットを加算
    pub(super) fn offset_line(&self, line: u32) -> u32 {
        line + self.line_offset.load(Ordering::Relaxed)
    }

    /// ASTノードからソーステキストを取得する
    pub(super) fn node_text(&self, node: Node, source: &str) -> String {
        source[node.byte_range()].to_string()
    }

    /// 文字列ノードから値を取得する（クォートを除去）
    pub(super) fn extract_string_value(&self, node: Node, source: &str) -> String {
        let text = self.node_text(node, source);
        text.trim_matches(|c| c == '"' || c == '\'').to_string()
    }

    /// 指定された行の直前にあるJSDocコメントを抽出する
    ///
    /// チェーン呼び出しの場合でもコンポーネント名の位置から正しくJSDocを検出する
    pub(super) fn extract_jsdoc_for_line(&self, target_line: usize, source: &str) -> Option<String> {
        let lines: Vec<&str> = source.lines().collect();

        // 対象行の直前の行から上に向かってJSDocコメントを探す
        // 空行はスキップする
        let mut search_line = target_line as i32 - 1;
        let mut jsdoc_end_line: Option<usize> = None;

        // まず、JSDocコメントの終了行（*/）を探す
        while search_line >= 0 {
            let line = lines.get(search_line as usize).map(|s| s.trim()).unwrap_or("");

            if line.is_empty() {
                search_line -= 1;
                continue;
            }

            if line.ends_with("*/") {
                jsdoc_end_line = Some(search_line as usize);
                break;
            }

            // 空行でもコメント終了でもない場合は、JSDocはない
            break;
        }

        let end_line = jsdoc_end_line?;

        // JSDocコメントの開始行（/**）を探す
        search_line = end_line as i32;
        while search_line >= 0 {
            let line = lines.get(search_line as usize).map(|s| s.trim()).unwrap_or("");

            if line.starts_with("/**") {
                // JSDocコメントを構築
                let jsdoc_lines: Vec<&str> = lines[search_line as usize..=end_line].to_vec();
                let jsdoc_text = jsdoc_lines.join("\n");
                return Some(self.parse_jsdoc(&jsdoc_text));
            }

            search_line -= 1;
        }

        None
    }

    /// JSDocコメントをパースして整形する
    pub(super) fn parse_jsdoc(&self, comment: &str) -> String {
        comment
            .lines()
            .map(|line| {
                let trimmed = line.trim();
                // 各行から /** */ * を除去
                let trimmed = trimmed.trim_start_matches("/**").trim_end_matches("*/");
                trimmed.trim_start_matches('*').trim()
            })
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 関数ノードからパラメータ名のリストを抽出する
    ///
    /// 認識パターン:
    /// - function(param1, param2) {}
    /// - (param1, param2) => {}
    pub(super) fn extract_function_params(&self, node: Node, source: &str) -> Option<Vec<String>> {
        let func_node = match node.kind() {
            "function_expression" | "arrow_function" | "function_declaration" => Some(node),
            "array" => {
                // DI配列: ['$scope', function($scope) {}]
                let mut cursor = node.walk();
                node.children(&mut cursor)
                    .find(|c| c.kind() == "function_expression" || c.kind() == "arrow_function")
            }
            _ => None,
        }?;

        let params_node = func_node.child_by_field_name("parameters")?;
        let mut params = Vec::new();

        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            if child.kind() == "identifier" {
                let param_name = self.node_text(child, source);
                params.push(param_name);
            }
        }

        if params.is_empty() {
            None
        } else {
            Some(params)
        }
    }
}

/// JavaScriptの予約語・キーワードかどうかを判定する
pub(super) fn is_common_keyword(name: &str) -> bool {
    matches!(
        name,
        "function" | "var" | "let" | "const" | "if" | "else" | "for" | "while"
            | "return" | "true" | "false" | "null" | "undefined" | "this"
            | "new" | "typeof" | "instanceof" | "in" | "of"
    )
}
