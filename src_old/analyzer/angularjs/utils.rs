use std::sync::atomic::Ordering;
use tree_sitter::Node;

use super::AngularJsAnalyzer;

impl AngularJsAnalyzer {
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
