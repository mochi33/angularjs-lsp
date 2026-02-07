//! $scope参照の収集

use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::directives::is_ng_directive;
use crate::index::HtmlScopeReference;

use super::HtmlAngularJsAnalyzer;

impl HtmlAngularJsAnalyzer {
    /// $scope参照を収集
    pub(super) fn collect_scope_references(&self, node: Node, source: &str, uri: &Url) {
        // 要素ノードの場合、AngularJSディレクティブをチェック
        if node.kind() == "element" {
            if let Some(start_tag) = self.find_child_by_kind(node, "start_tag") {
                self.extract_scope_references_from_tag(start_tag, source, uri);
            }
        }

        // 自己終了タグ（<input ... />等）の場合
        if node.kind() == "self_closing_tag" {
            self.extract_scope_references_from_tag(node, source, uri);
        }

        // text内の{{interpolation}}をチェック
        if node.kind() == "text" {
            let text = self.node_text(node, source);
            self.extract_interpolation_references(&text, node, source, uri);
        }

        // 子ノードを再帰的に処理
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.collect_scope_references(child, source, uri);
        }
    }

    /// タグの属性からスコープ参照を抽出
    fn extract_scope_references_from_tag(&self, start_tag: Node, source: &str, uri: &Url) {
        let mut cursor = start_tag.walk();
        for child in start_tag.children(&mut cursor) {
            if child.kind() == "attribute" {
                if let Some(name_node) = self.find_child_by_kind(child, "attribute_name") {
                    let attr_name = self.node_text(name_node, source);

                    if let Some(value_node) = self.find_child_by_kind(child, "quoted_attribute_value") {
                        let raw_value = self.node_text(value_node, source);
                        let value = raw_value.trim_matches(|c| c == '"' || c == '\'');

                        // 属性値の開始位置（クォートの後）- UTF-16変換
                        let value_start_line = value_node.start_position().row as usize;
                        let value_byte_col = value_node.start_position().column + 1; // +1 for quote
                        let value_start_col = self.byte_col_to_utf16_col(source, value_start_line, value_byte_col);

                        if is_ng_directive(&attr_name) {
                            // ngディレクティブ: 属性値全体をAngular式として解析
                            let property_paths = self.parse_angular_expression(value, &attr_name);
                            self.register_scope_references(uri, value, &property_paths, value_start_line as u32, value_start_col);
                        } else {
                            // 非ディレクティブ属性: インターポレーションのみを抽出
                            self.extract_interpolation_references_from_attribute(value, value_node, source, uri);
                        }
                    }
                }
            }
        }
    }

    /// スコープ参照を登録（共通処理）- UTF-16対応
    fn register_scope_references(
        &self,
        uri: &Url,
        value: &str,
        property_paths: &[String],
        value_start_line: u32,
        value_start_col: u32,  // UTF-16コードユニット単位
    ) {
        for property_path in property_paths {
            // ローカル変数の場合はスキップ（HtmlScopeReferenceではなくHtmlLocalVariableReferenceとして登録済み）
            let base_name = property_path.split('.').next().unwrap_or(property_path);
            if self.index.find_local_variable_definition(uri, base_name, value_start_line).is_some() {
                continue;
            }

            // alias.property形式の場合、aliasまたはフォームバインディングが有効かチェック
            // どちらでもない場合はスキップ（単純な識別子だけを登録）
            if property_path.contains('.') {
                let parts: Vec<&str> = property_path.splitn(2, '.').collect();
                if parts.len() == 2 {
                    let potential_alias = parts[0];
                    // このaliasが有効かチェック
                    let is_alias = self.index.resolve_controller_by_alias(uri, value_start_line, potential_alias).is_some();
                    // フォームバインディングかチェック
                    let is_form_binding = self.index.find_form_binding_definition(uri, potential_alias, value_start_line).is_some();
                    if !is_alias && !is_form_binding {
                        // aliasでもフォームバインディングでもないのでスキップ
                        continue;
                    }
                }
            }

            // 属性値内で識別子のすべての出現位置を検索
            let positions = self.find_identifier_positions(value, property_path);

            for (byte_offset, byte_len) in positions {
                // マルチライン属性値の場合、実際の行と列を計算（UTF-16対応）
                let before_identifier = &value[..byte_offset];
                let identifier_text = &value[byte_offset..byte_offset + byte_len];
                let newline_count = before_identifier.matches('\n').count();

                let start_line = value_start_line + newline_count as u32;
                let start_col = if newline_count == 0 {
                    // 改行がない場合、UTF-16オフセットを加算
                    value_start_col + self.byte_offset_to_utf16_offset(before_identifier, before_identifier.len()) as u32
                } else {
                    // 改行がある場合、最後の改行以降のテキストをUTF-16に変換
                    let last_newline_pos = before_identifier.rfind('\n').unwrap();
                    let after_newline = &before_identifier[last_newline_pos + 1..];
                    self.byte_offset_to_utf16_offset(after_newline, after_newline.len()) as u32
                };
                let end_line = start_line; // 識別子は1行内と仮定
                let end_col = start_col + identifier_text.chars().map(|c| c.len_utf16()).sum::<usize>() as u32;

                // HtmlScopeReferenceを登録（コントローラー解決は参照検索時に行う）
                let html_reference = HtmlScopeReference {
                    property_path: property_path.clone(),
                    uri: uri.clone(),
                    start_line,
                    start_col,
                    end_line,
                    end_col,
                };
                self.index.add_html_scope_reference(html_reference);
            }
        }
    }

    /// 属性値内のインターポレーションからスコープ参照を抽出（UTF-16対応）
    fn extract_interpolation_references_from_attribute(
        &self,
        value: &str,
        value_node: Node,
        source: &str,
        uri: &Url,
    ) {
        let (start_symbol, end_symbol) = self.get_interpolate_symbols();
        let start_len = start_symbol.len();
        let end_len = end_symbol.len();

        let value_start_line = value_node.start_position().row as usize;
        let value_byte_col = value_node.start_position().column + 1; // +1 for quote
        let value_start_col = self.byte_col_to_utf16_col(source, value_start_line, value_byte_col);

        let mut start = 0;
        while let Some(open_idx) = value[start..].find(&start_symbol) {
            let abs_open = start + open_idx;
            if let Some(close_idx) = value[abs_open..].find(&end_symbol) {
                let abs_close = abs_open + close_idx;
                let expr = &value[abs_open + start_len..abs_close];
                let expr_trimmed = expr.trim();

                // 式の開始位置（{{ の後、トリム前の空白を考慮）- バイトオフセット
                let expr_start_byte_offset = abs_open + start_len + (expr.len() - expr.trim_start().len());

                let property_paths = self.parse_angular_expression(expr_trimmed, "interpolation");

                // 式内での位置を計算（UTF-16対応）
                let before_expr = &value[..expr_start_byte_offset];
                let newline_count = before_expr.matches('\n').count();
                let expr_line = value_start_line + newline_count;

                let expr_col = if newline_count == 0 {
                    value_start_col + self.byte_offset_to_utf16_offset(before_expr, before_expr.len()) as u32
                } else {
                    let last_newline_pos = before_expr.rfind('\n').unwrap();
                    let after_newline = &value[last_newline_pos + 1..expr_start_byte_offset];
                    self.byte_offset_to_utf16_offset(after_newline, after_newline.len()) as u32
                };

                // 式内でのプロパティパスの位置を登録
                for property_path in &property_paths {
                    // ローカル変数の場合はスキップ
                    let base_name = property_path.split('.').next().unwrap_or(property_path);
                    if self.index.find_local_variable_definition(uri, base_name, expr_line as u32).is_some() {
                        continue;
                    }

                    // alias.property形式の場合、aliasまたはフォームバインディングが有効かチェック
                    if property_path.contains('.') {
                        let parts: Vec<&str> = property_path.splitn(2, '.').collect();
                        if parts.len() == 2 {
                            let potential_alias = parts[0];
                            let is_alias = self.index.resolve_controller_by_alias(uri, expr_line as u32, potential_alias).is_some();
                            let is_form_binding = self.index.find_form_binding_definition(uri, potential_alias, expr_line as u32).is_some();
                            if !is_alias && !is_form_binding {
                                continue;
                            }
                        }
                    }

                    // 式内で識別子のすべての出現位置を検索
                    let positions = self.find_identifier_positions(expr_trimmed, property_path);

                    for (byte_offset, byte_len) in positions {
                        let before_identifier = &expr_trimmed[..byte_offset];
                        let identifier_text = &expr_trimmed[byte_offset..byte_offset + byte_len];
                        let newline_count_in_expr = before_identifier.matches('\n').count();

                        let start_line = expr_line + newline_count_in_expr;
                        let start_col = if newline_count_in_expr == 0 {
                            expr_col + self.byte_offset_to_utf16_offset(before_identifier, before_identifier.len()) as u32
                        } else {
                            let last_newline_pos = before_identifier.rfind('\n').unwrap();
                            let after_newline = &before_identifier[last_newline_pos + 1..];
                            self.byte_offset_to_utf16_offset(after_newline, after_newline.len()) as u32
                        };
                        let end_line = start_line;
                        let end_col = start_col + identifier_text.chars().map(|c| c.len_utf16()).sum::<usize>() as u32;

                        let html_reference = HtmlScopeReference {
                            property_path: property_path.clone(),
                            uri: uri.clone(),
                            start_line: start_line as u32,
                            start_col,
                            end_line: end_line as u32,
                            end_col,
                        };
                        self.index.add_html_scope_reference(html_reference);
                    }
                }

                start = abs_close + end_len;
            } else {
                break;
            }
        }
    }

    /// 文字列内で識別子のすべての出現位置を検索（単語境界を考慮）
    pub(super) fn find_identifier_positions(&self, text: &str, identifier: &str) -> Vec<(usize, usize)> {
        let mut positions = Vec::new();
        let mut start = 0;

        while start < text.len() {
            // 安全にスライスするため、文字境界かチェック
            if !text.is_char_boundary(start) {
                start += 1;
                continue;
            }

            let Some(offset) = text[start..].find(identifier) else {
                break;
            };

            let abs_offset = start + offset;
            let end_offset = abs_offset + identifier.len();

            // end_offsetが文字境界でない場合はスキップ
            if end_offset > text.len() || !text.is_char_boundary(end_offset) {
                start = abs_offset + 1;
                continue;
            }

            // 単語境界をチェック（識別子の前後が識別子文字でないこと）
            let before_ok = abs_offset == 0
                || !text[..abs_offset]
                    .chars()
                    .last()
                    .map(|c| c.is_alphanumeric() || c == '_' || c == '$')
                    .unwrap_or(false);

            let after_ok = end_offset >= text.len()
                || !text[end_offset..]
                    .chars()
                    .next()
                    .map(|c| c.is_alphanumeric() || c == '_' || c == '$')
                    .unwrap_or(false);

            if before_ok && after_ok {
                positions.push((abs_offset, identifier.len()));
            }

            start = abs_offset + 1;
        }

        positions
    }

    /// マルチライン文字列内でのオフセットから、実際の行オフセットと列を計算
    /// 戻り値: (行オフセット, 列)
    pub(super) fn calculate_position_in_multiline(&self, text: &str, offset: usize, initial_col: usize) -> (usize, usize) {
        let before_offset = &text[..offset];
        let newline_count = before_offset.matches('\n').count();

        if newline_count == 0 {
            // 改行がない場合、初期列にオフセットを加える
            (0, initial_col + offset)
        } else {
            // 最後の改行以降の文字数が列番号
            let last_newline_pos = before_offset.rfind('\n').unwrap();
            let col = offset - last_newline_pos - 1; // -1 for the newline char itself
            (newline_count, col)
        }
    }

    /// バイトオフセットの列位置をUTF-16コードユニット単位の列位置に変換
    pub(super) fn byte_col_to_utf16_col(&self, source: &str, line: usize, byte_col: usize) -> u32 {
        // 該当行を取得
        if let Some(line_content) = source.lines().nth(line) {
            // バイト位置までの文字をUTF-16コードユニット数でカウント
            let mut utf16_col = 0u32;
            let mut byte_count = 0usize;
            for c in line_content.chars() {
                if byte_count >= byte_col {
                    break;
                }
                byte_count += c.len_utf8();
                utf16_col += c.len_utf16() as u32;
            }
            utf16_col
        } else {
            byte_col as u32
        }
    }

    /// テキスト内でのバイトオフセットからUTF-16コードユニット数を計算
    pub(super) fn byte_offset_to_utf16_offset(&self, text: &str, byte_offset: usize) -> usize {
        let before = &text[..byte_offset.min(text.len())];
        before.chars().map(|c| c.len_utf16()).sum()
    }

    /// interpolation（デフォルト: {{...}}）からスコープ参照を抽出
    fn extract_interpolation_references(&self, text: &str, node: Node, source: &str, uri: &Url) {
        let (start_symbol, end_symbol) = self.get_interpolate_symbols();
        let start_len = start_symbol.len();
        let end_len = end_symbol.len();

        let node_start_line = node.start_position().row as usize;
        let node_start_byte_col = node.start_position().column;

        // ノードの開始列位置をUTF-16に変換
        let node_start_col = self.byte_col_to_utf16_col(source, node_start_line, node_start_byte_col);

        let mut start = 0;
        while let Some(open_idx) = text[start..].find(&start_symbol) {
            let abs_open = start + open_idx;
            if let Some(close_idx) = text[abs_open..].find(&end_symbol) {
                let abs_close = abs_open + close_idx;
                let expr = &text[abs_open + start_len..abs_close];
                let expr_trimmed = expr.trim();

                // 式の開始位置（{{ の後、トリム前の空白を考慮）- バイトオフセット
                let expr_start_byte_offset = abs_open + start_len + (expr.len() - expr.trim_start().len());

                let property_paths = self.parse_angular_expression(expr_trimmed, "interpolation");

                // 式内での位置を計算（マルチライン対応、UTF-16変換）
                let before_expr = &text[..expr_start_byte_offset];
                let newline_count = before_expr.matches('\n').count();
                let expr_line = node_start_line + newline_count;

                let expr_col = if newline_count == 0 {
                    // 改行がない場合、テキスト内のオフセットをUTF-16に変換して加算
                    let utf16_offset = self.byte_offset_to_utf16_offset(text, expr_start_byte_offset);
                    node_start_col + utf16_offset as u32
                } else {
                    // 改行がある場合、最後の改行以降のテキストをUTF-16に変換
                    let last_newline_pos = before_expr.rfind('\n').unwrap();
                    let after_newline = &text[last_newline_pos + 1..expr_start_byte_offset];
                    self.byte_offset_to_utf16_offset(after_newline, after_newline.len()) as u32
                };

                for property_path in property_paths {
                    // ローカル変数の場合はスキップ
                    let base_name = property_path.split('.').next().unwrap_or(&property_path);
                    if self.index.find_local_variable_definition(uri, base_name, expr_line as u32).is_some() {
                        continue;
                    }

                    // alias.property形式の場合、aliasまたはフォームバインディングが有効かチェック
                    // どちらでもない場合はスキップ（単純な識別子だけを登録）
                    if property_path.contains('.') {
                        let parts: Vec<&str> = property_path.splitn(2, '.').collect();
                        if parts.len() == 2 {
                            let potential_alias = parts[0];
                            // このaliasが有効かチェック
                            let is_alias = self.index.resolve_controller_by_alias(uri, expr_line as u32, potential_alias).is_some();
                            // フォームバインディングかチェック
                            let is_form_binding = self.index.find_form_binding_definition(uri, potential_alias, expr_line as u32).is_some();
                            if !is_alias && !is_form_binding {
                                // aliasでもフォームバインディングでもないのでスキップ
                                continue;
                            }
                        }
                    }

                    // 式内で識別子のすべての出現位置を検索
                    let positions = self.find_identifier_positions(expr_trimmed, &property_path);

                    for (byte_offset, byte_len) in positions {
                        // 式内でのUTF-16オフセットを計算
                        let before_identifier = &expr_trimmed[..byte_offset];
                        let identifier_text = &expr_trimmed[byte_offset..byte_offset + byte_len];
                        let newline_count_in_expr = before_identifier.matches('\n').count();

                        let start_line = expr_line + newline_count_in_expr;
                        let start_col = if newline_count_in_expr == 0 {
                            expr_col + self.byte_offset_to_utf16_offset(before_identifier, before_identifier.len()) as u32
                        } else {
                            let last_newline_pos = before_identifier.rfind('\n').unwrap();
                            let after_newline = &before_identifier[last_newline_pos + 1..];
                            self.byte_offset_to_utf16_offset(after_newline, after_newline.len()) as u32
                        };
                        let end_line = start_line;
                        let end_col = start_col + identifier_text.chars().map(|c| c.len_utf16()).sum::<usize>() as u32;

                        // HtmlScopeReferenceを登録（コントローラー解決は参照検索時に行う）
                        let html_reference = HtmlScopeReference {
                            property_path: property_path.clone(),
                            uri: uri.clone(),
                            start_line: start_line as u32,
                            start_col,
                            end_line: end_line as u32,
                            end_col,
                        };
                        self.index.add_html_scope_reference(html_reference);
                    }
                }

                start = abs_close + end_len;
            } else {
                break;
            }
        }
    }
}