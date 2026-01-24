//! Angular式のパースとコンテキスト判定

use super::directives::is_ng_directive;
use super::JsParser;

use super::HtmlAngularJsAnalyzer;

impl HtmlAngularJsAnalyzer {
    /// AngularJS式からプロパティパスを抽出（tree-sitter使用）
    pub(super) fn parse_angular_expression(&self, expr: &str, directive: &str) -> Vec<String> {
        let mut local_vars: Vec<String> = Vec::new();

        // ng-repeat: "item in items" or "(key, value) in items" -> ローカル変数を抽出
        let expr_to_parse = if directive.contains("ng-repeat") || directive.contains("data-ng-repeat") {
            if let Some(in_idx) = expr.find(" in ") {
                let iter_part = expr[..in_idx].trim();
                // (key, value) 形式
                if iter_part.starts_with('(') && iter_part.ends_with(')') {
                    let inner = &iter_part[1..iter_part.len()-1];
                    for var in inner.split(',') {
                        local_vars.push(var.trim().to_string());
                    }
                } else {
                    // item 形式
                    local_vars.push(iter_part.to_string());
                }
                // "in"の後の部分だけをパース
                &expr[in_idx + 4..]
            } else {
                expr
            }
        } else {
            expr
        };

        // フィルター部分を除去（AngularJSフィルターはJS構文ではない）
        // 注意: || はJavaScriptの演算子なので、単独の | のみをフィルター区切りとして扱う
        let expr_to_parse = self.remove_angular_filters(expr_to_parse);

        // tree-sitter-javascriptで式をパース
        let mut parser = JsParser::new();
        let mut identifiers = Vec::new();

        if let Some(tree) = parser.parse(expr_to_parse) {
            self.collect_identifiers_from_expr(tree.root_node(), expr_to_parse, &mut identifiers);
        }

        // ローカル変数とAngularキーワードを除外
        identifiers
            .into_iter()
            .filter(|name| !local_vars.contains(name) && !self.is_angular_keyword(name))
            .collect()
    }

    /// 式のASTから識別子を収集
    fn collect_identifiers_from_expr(&self, node: tree_sitter::Node, source: &str, identifiers: &mut Vec<String>) {
        match node.kind() {
            // member_expression:
            // - user.name -> "user" (直接のスコープ変数)
            // - alias.property -> "alias.property" (controller as alias構文)
            // 両方の形式を収集し、参照解決時にaliasかどうかをチェック
            "member_expression" => {
                if let Some(object) = node.child_by_field_name("object") {
                    // ネストしたmember_expression (a.b.c) の場合
                    if object.kind() == "member_expression" {
                        // 最初のオブジェクト（a）を取得
                        self.collect_identifiers_from_expr(object, source, identifiers);
                    } else if object.kind() == "identifier" {
                        let obj_name = self.node_text(object, source);
                        // 直接のプロパティを取得（controller as alias構文のサポート）
                        if let Some(property) = node.child_by_field_name("property") {
                            let prop_name = self.node_text(property, source);
                            let member_path = format!("{}.{}", obj_name, prop_name);
                            if !identifiers.contains(&member_path) {
                                identifiers.push(member_path);
                            }
                        }
                        // オブジェクト名自体も追加（通常のスコープ変数として）
                        // 注: 両方を追加することで、alias.propertyと$scope.userの両方に対応
                        if !identifiers.contains(&obj_name) {
                            identifiers.push(obj_name);
                        }
                    } else {
                        // call_expression等の場合は子を探索
                        self.collect_identifiers_from_expr(object, source, identifiers);
                    }
                }
                // argumentsがある場合（メソッド呼び出しの引数など）
                // member_expressionの子ノードも探索
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() != "identifier" && child.kind() != "property_identifier" {
                            self.collect_identifiers_from_expr(child, source, identifiers);
                        }
                    }
                }
            }
            // call_expression: save(user) -> "save"と"user"を抽出
            "call_expression" => {
                if let Some(func) = node.child_by_field_name("function") {
                    self.collect_identifiers_from_expr(func, source, identifiers);
                }
                if let Some(args) = node.child_by_field_name("arguments") {
                    self.collect_identifiers_from_expr(args, source, identifiers);
                }
            }
            // 単独の識別子
            "identifier" => {
                let name = self.node_text(node, source);
                if !identifiers.contains(&name) {
                    identifiers.push(name);
                }
            }
            // その他のノードは子を再帰的に探索
            _ => {
                // named_childrenではなく全ての子ノードを探索
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        self.collect_identifiers_from_expr(child, source, identifiers);
                    }
                }
            }
        }
    }

    /// AngularJSフィルターを除去（|| は演算子なので保持）
    fn remove_angular_filters<'a>(&self, expr: &'a str) -> &'a str {
        let bytes = expr.as_bytes();
        for i in 0..bytes.len() {
            if bytes[i] == b'|' {
                // || の場合はスキップ（JavaScript論理OR演算子）
                if i + 1 < bytes.len() && bytes[i + 1] == b'|' {
                    continue;
                }
                // 前の文字が | でないことも確認（|| の2文字目をスキップ）
                if i > 0 && bytes[i - 1] == b'|' {
                    continue;
                }
                // 単独の | はフィルター区切り
                return expr[..i].trim();
            }
        }
        expr.trim()
    }

    /// AngularJSのキーワードかどうか
    fn is_angular_keyword(&self, name: &str) -> bool {
        matches!(
            name,
            "true" | "false" | "null" | "undefined" |
            "$index" | "$first" | "$last" | "$middle" | "$odd" | "$even" |
            "track" | "by" | "in" | "as" |
            // ng-repeatでよく使われるローカル変数名
            "item" | "key" | "value" | "i" | "idx" |
            // JavaScript組み込み
            "console" | "window" | "document" | "Math" | "JSON" | "Array" | "Object" | "String" | "Number"
        )
    }

    /// カーソル位置がAngularディレクティブまたはinterpolation内にあるかを判定
    /// 戻り値: true = Angular コンテキスト内（$scope補完が必要）
    pub fn is_in_angular_context(&self, source: &str, line: u32, col: u32) -> bool {
        let lines: Vec<&str> = source.lines().collect();
        if (line as usize) >= lines.len() {
            return false;
        }

        let current_line = lines[line as usize];
        let col = col as usize;
        if col > current_line.len() {
            return false;
        }

        let before_cursor = &current_line[..col];

        // 1. interpolation内かチェック（{{ ... }}）
        let (start_symbol, end_symbol) = self.get_interpolate_symbols();
        if let Some(open_idx) = before_cursor.rfind(&start_symbol) {
            // 開き記号の後に閉じ記号がないかチェック
            let after_open = &before_cursor[open_idx + start_symbol.len()..];
            if !after_open.contains(&end_symbol) {
                return true;
            }
        }

        // 2. AngularJSディレクティブ属性内かチェック
        // ng-if="...", ng-model="...", ng-click="..." など
        // ダブルクォートパターンをチェック
        if let Some(eq_idx) = before_cursor.rfind("=\"") {
            let after_eq = &before_cursor[eq_idx + 2..];
            // 属性値の閉じクォートがない場合、属性値内にいる
            if !after_eq.contains('"') {
                // 属性名を抽出（`="` の前の部分）
                if let Some(attr_name) = Self::extract_attr_name(&before_cursor[..eq_idx]) {
                    if is_ng_directive(attr_name) {
                        return true;
                    }
                }
            }
        }

        // シングルクォートパターンをチェック
        if let Some(eq_idx) = before_cursor.rfind("='") {
            let after_eq = &before_cursor[eq_idx + 2..];
            // 属性値の閉じクォートがない場合、属性値内にいる
            if !after_eq.contains('\'') {
                // 属性名を抽出（`='` の前の部分）
                if let Some(attr_name) = Self::extract_attr_name(&before_cursor[..eq_idx]) {
                    if is_ng_directive(attr_name) {
                        return true;
                    }
                }
            }
        }

        false
    }

    /// 文字列の末尾から属性名を抽出
    fn extract_attr_name(s: &str) -> Option<&str> {
        // 末尾から属性名の開始位置を探す（スペースまたは < まで）
        let start = s.rfind(|c: char| c.is_whitespace() || c == '<')
            .map(|i| i + 1)
            .unwrap_or(0);
        let attr_name = &s[start..];
        if attr_name.is_empty() {
            None
        } else {
            Some(attr_name)
        }
    }

    /// カーソル位置がHTMLタグ名または属性名の位置かを判定（ディレクティブ補完用）
    /// 属性値の中ではfalseを返す
    /// 戻り値: Some((prefix, is_tag_name)) - prefix: 入力中の文字列, is_tag_name: タグ名位置かどうか
    pub fn get_directive_completion_context(&self, source: &str, line: u32, col: u32) -> Option<(String, bool)> {
        let lines: Vec<&str> = source.lines().collect();
        if (line as usize) >= lines.len() {
            return None;
        }

        let current_line = lines[line as usize];
        let col = col as usize;
        if col > current_line.len() {
            return None;
        }

        let before_cursor = &current_line[..col];

        // 開いているタグ `<` を逆方向に探す
        let last_open_tag = before_cursor.rfind('<');
        let last_close_tag = before_cursor.rfind('>');

        // `>` が `<` より後にある場合、タグの外にいる
        if let (Some(open_idx), Some(close_idx)) = (last_open_tag, last_close_tag) {
            if close_idx > open_idx {
                return None;
            }
        }

        // `<` がない場合、タグの外
        let open_idx = last_open_tag?;

        // タグ内のテキスト（`<` の後からカーソルまで）
        let tag_content = &before_cursor[open_idx + 1..];

        // 閉じタグ `</` の場合は補完しない
        if tag_content.starts_with('/') {
            return None;
        }

        // 属性値の中にいるかチェック
        // クォートの数をカウント（奇数なら属性値の中）
        let double_quote_count = tag_content.matches('"').count();
        let single_quote_count = tag_content.matches('\'').count();

        // どちらかのクォートが奇数なら、属性値の中にいる
        if double_quote_count % 2 == 1 || single_quote_count % 2 == 1 {
            return None;
        }

        // タグ名位置かどうかを判定
        // スペースがなければタグ名、あれば属性名
        if !tag_content.contains(char::is_whitespace) {
            // タグ名位置
            Some((tag_content.to_string(), true))
        } else {
            // 属性名位置 - 最後のスペース後の文字列を取得
            // ただし `=` の後にいる場合（属性値を開始しようとしている場合）は除外
            let last_space_idx = tag_content.rfind(char::is_whitespace)?;
            let attr_part = &tag_content[last_space_idx + 1..];

            // `=` の後の場合、属性値の開始位置なので補完しない
            if tag_content.trim_end().ends_with('=') {
                return None;
            }

            // `=` が含まれている場合（`attr=`の後の補完の場合は除外）
            if attr_part.contains('=') {
                return None;
            }

            Some((attr_part.to_string(), false))
        }
    }
}