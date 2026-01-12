//! HTML内のローカル変数（ng-init, ng-repeat由来）の解析

use std::collections::HashMap;

use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::directives::is_ng_directive;
use super::variable_parser::{parse_ng_init_expression, parse_ng_repeat_expression};
use super::HtmlAngularJsAnalyzer;
use crate::index::{HtmlLocalVariable, HtmlLocalVariableReference};

impl HtmlAngularJsAnalyzer {
    /// ローカル変数定義を収集（Pass 4a）
    pub(super) fn collect_local_variable_definitions(&self, node: Node, source: &str, uri: &Url) {
        // element または self_closing_tag からローカル変数を抽出
        let tag_node = if node.kind() == "element" {
            self.find_child_by_kind(node, "start_tag")
        } else if node.kind() == "self_closing_tag" {
            Some(node)
        } else {
            None
        };

        if let Some(tag) = tag_node {
            // 要素のスコープ範囲
            let scope_start_line = node.start_position().row as u32;
            let scope_end_line = node.end_position().row as u32;

            // ng-repeatからローカル変数を抽出
            self.extract_ng_repeat_variable_definitions(
                tag,
                source,
                uri,
                scope_start_line,
                scope_end_line,
            );

            // ng-initからローカル変数を抽出
            self.extract_ng_init_variable_definitions(
                tag,
                source,
                uri,
                scope_start_line,
                scope_end_line,
            );
        }

        // 子ノードを再帰的に処理
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.collect_local_variable_definitions(child, source, uri);
        }
    }

    /// ng-repeatから変数定義を抽出
    fn extract_ng_repeat_variable_definitions(
        &self,
        start_tag: Node,
        source: &str,
        uri: &Url,
        scope_start_line: u32,
        scope_end_line: u32,
    ) {
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

                            // 属性値の開始位置（クォートの後）- UTF-16変換
                            let value_start_line = value_node.start_position().row as usize;
                            let value_byte_col = value_node.start_position().column + 1;
                            let value_start_col = self.byte_col_to_utf16_col(source, value_start_line, value_byte_col);

                            // 共通パーサーを使用
                            let parsed_vars = parse_ng_repeat_expression(value);

                            for var in parsed_vars {
                                // バイトオフセットからUTF-16位置を計算
                                let before_var = &value[..var.offset];
                                let var_text = &value[var.offset..var.offset + var.len];
                                let newline_count = before_var.matches('\n').count();

                                let name_start_line = value_start_line as u32 + newline_count as u32;
                                let name_start_col = if newline_count == 0 {
                                    value_start_col + self.byte_offset_to_utf16_offset(before_var, before_var.len()) as u32
                                } else {
                                    let last_newline_pos = before_var.rfind('\n').unwrap();
                                    let after_newline = &before_var[last_newline_pos + 1..];
                                    self.byte_offset_to_utf16_offset(after_newline, after_newline.len()) as u32
                                };
                                let name_end_line = name_start_line;
                                let name_end_col = name_start_col + var_text.chars().map(|c| c.len_utf16()).sum::<usize>() as u32;

                                let variable = HtmlLocalVariable {
                                    name: var.name,
                                    source: var.source,
                                    uri: uri.clone(),
                                    scope_start_line,
                                    scope_end_line,
                                    name_start_line,
                                    name_start_col,
                                    name_end_line,
                                    name_end_col,
                                };
                                self.index.add_html_local_variable(variable);
                            }
                        }
                    }
                }
            }
        }
    }

    /// ng-initから変数定義を抽出
    fn extract_ng_init_variable_definitions(
        &self,
        start_tag: Node,
        source: &str,
        uri: &Url,
        scope_start_line: u32,
        scope_end_line: u32,
    ) {
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

                            // 属性値の開始位置（クォートの後）- UTF-16変換
                            let value_start_line = value_node.start_position().row as usize;
                            let value_byte_col = value_node.start_position().column + 1;
                            let value_start_col = self.byte_col_to_utf16_col(source, value_start_line, value_byte_col);

                            // 共通パーサーを使用
                            let parsed_vars = parse_ng_init_expression(value);

                            for var in parsed_vars {
                                // バイトオフセットからUTF-16位置を計算
                                let before_var = &value[..var.offset];
                                let var_text = &value[var.offset..var.offset + var.len];
                                let newline_count = before_var.matches('\n').count();

                                let name_start_line = value_start_line as u32 + newline_count as u32;
                                let name_start_col = if newline_count == 0 {
                                    value_start_col + self.byte_offset_to_utf16_offset(before_var, before_var.len()) as u32
                                } else {
                                    let last_newline_pos = before_var.rfind('\n').unwrap();
                                    let after_newline = &before_var[last_newline_pos + 1..];
                                    self.byte_offset_to_utf16_offset(after_newline, after_newline.len()) as u32
                                };
                                let name_end_line = name_start_line;
                                let name_end_col = name_start_col + var_text.chars().map(|c| c.len_utf16()).sum::<usize>() as u32;

                                let variable = HtmlLocalVariable {
                                    name: var.name,
                                    source: var.source,
                                    uri: uri.clone(),
                                    scope_start_line,
                                    scope_end_line,
                                    name_start_line,
                                    name_start_col,
                                    name_end_line,
                                    name_end_col,
                                };
                                self.index.add_html_local_variable(variable);
                            }
                        }
                    }
                }
            }
        }
    }

    /// ローカル変数参照を収集（Pass 4b）
    /// 現在有効なローカル変数のスコープを追跡しながら収集
    pub(super) fn collect_local_variable_references(
        &self,
        node: Node,
        source: &str,
        uri: &Url,
        active_scopes: &mut HashMap<String, (u32, u32)>, // var_name -> (scope_start, scope_end)
    ) {
        // 要素ノードの場合、新しいローカル変数スコープを追加
        let mut new_vars: Vec<String> = Vec::new();

        // element または self_closing_tag を処理
        let is_element_or_self_closing = node.kind() == "element" || node.kind() == "self_closing_tag";

        if is_element_or_self_closing {
            let scope_start_line = node.start_position().row as u32;
            let scope_end_line = node.end_position().row as u32;

            // このノードで定義されているローカル変数を取得
            let local_vars = self.index.get_local_variables_at(uri, scope_start_line);
            for var in &local_vars {
                if var.scope_start_line == scope_start_line && var.scope_end_line == scope_end_line
                {
                    new_vars.push(var.name.clone());
                    active_scopes
                        .insert(var.name.clone(), (var.scope_start_line, var.scope_end_line));
                }
            }

            // ディレクティブ属性内の参照を収集
            let tag_node = if node.kind() == "element" {
                self.find_child_by_kind(node, "start_tag")
            } else {
                Some(node) // self_closing_tag の場合はノード自体
            };

            if let Some(tag) = tag_node {
                self.extract_local_variable_references_from_tag(
                    tag,
                    source,
                    uri,
                    active_scopes,
                );
            }
        }

        // テキストノード内のinterpolationから参照を収集
        if node.kind() == "text" {
            let text = self.node_text(node, source);
            self.extract_local_variable_references_from_interpolation_utf16(
                &text,
                node,
                source,
                uri,
                active_scopes,
            );
        }

        // 子ノードを再帰的に処理
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.collect_local_variable_references(child, source, uri, active_scopes);
        }

        // このノードで追加したスコープを削除
        for var_name in new_vars {
            active_scopes.remove(&var_name);
        }
    }

    /// タグの属性からローカル変数参照を抽出
    fn extract_local_variable_references_from_tag(
        &self,
        start_tag: Node,
        source: &str,
        uri: &Url,
        active_scopes: &HashMap<String, (u32, u32)>,
    ) {
        let mut cursor = start_tag.walk();
        for child in start_tag.children(&mut cursor) {
            if child.kind() == "attribute" {
                if let Some(name_node) = self.find_child_by_kind(child, "attribute_name") {
                    let attr_name = self.node_text(name_node, source);

                    // ng-repeatとng-initは変数定義なのでスキップ（ただし右辺は参照としてチェック）
                    if attr_name == "ng-repeat"
                        || attr_name == "data-ng-repeat"
                        || attr_name == "ng-init"
                        || attr_name == "data-ng-init"
                    {
                        // ng-repeatの右辺（"in"の後）とng-initの右辺（=の後）のみチェック
                        if let Some(value_node) =
                            self.find_child_by_kind(child, "quoted_attribute_value")
                        {
                            let raw_value = self.node_text(value_node, source);
                            let value = raw_value.trim_matches(|c| c == '"' || c == '\'');
                            let value_start_line = value_node.start_position().row as usize;
                            let value_byte_col = value_node.start_position().column + 1;
                            let value_start_col = self.byte_col_to_utf16_col(source, value_start_line, value_byte_col);

                            // ng-repeatの場合は"in"の後の部分のみ
                            let expr_to_check =
                                if attr_name.contains("ng-repeat") {
                                    if let Some(in_idx) = value.find(" in ") {
                                        // track byを除去
                                        let after_in = &value[in_idx + 4..];
                                        let after_in =
                                            if let Some(track_idx) = after_in.find(" track ") {
                                                &after_in[..track_idx]
                                            } else {
                                                after_in
                                            };
                                        // フィルタを除去
                                        let after_in = after_in.split('|').next().unwrap_or(after_in);
                                        // UTF-16オフセットを計算
                                        let before_in = &value[..in_idx + 4];
                                        let utf16_offset = self.byte_offset_to_utf16_offset(before_in, before_in.len());
                                        Some((after_in.to_string(), utf16_offset))
                                    } else {
                                        None
                                    }
                                } else {
                                    // ng-initは各ステートメントの右辺
                                    None // 複雑なのでスキップ（将来的に対応可能）
                                };

                            if let Some((expr, utf16_offset_in_value)) = expr_to_check {
                                self.check_and_register_local_var_references_utf16(
                                    &expr,
                                    uri,
                                    value_start_line as u32,
                                    value_start_col + utf16_offset_in_value as u32,
                                    active_scopes,
                                );
                            }
                        }
                        continue;
                    }

                    if let Some(value_node) =
                        self.find_child_by_kind(child, "quoted_attribute_value")
                    {
                        let raw_value = self.node_text(value_node, source);
                        let value = raw_value.trim_matches(|c| c == '"' || c == '\'');
                        let value_start_line = value_node.start_position().row as usize;
                        let value_byte_col = value_node.start_position().column + 1;
                        let value_start_col = self.byte_col_to_utf16_col(source, value_start_line, value_byte_col);

                        if is_ng_directive(&attr_name) {
                            // ngディレクティブ: 属性値全体をAngular式として解析
                            // フィルタを除去
                            let expr = value.split('|').next().unwrap_or(value);

                            self.check_and_register_local_var_references_utf16(
                                expr,
                                uri,
                                value_start_line as u32,
                                value_start_col,
                                active_scopes,
                            );
                        } else {
                            // 非ディレクティブ属性: インターポレーションのみを抽出
                            self.extract_local_variable_references_from_attribute_interpolation_utf16(
                                value,
                                uri,
                                value_start_line,
                                value_start_col,
                                active_scopes,
                            );
                        }
                    }
                }
            }
        }
    }

    /// 属性値内のインターポレーションからローカル変数参照を抽出（UTF-16対応版）
    fn extract_local_variable_references_from_attribute_interpolation_utf16(
        &self,
        value: &str,
        uri: &Url,
        value_start_line: usize,
        value_start_col: u32, // UTF-16
        active_scopes: &HashMap<String, (u32, u32)>,
    ) {
        let (start_symbol, end_symbol) = self.get_interpolate_symbols();
        let start_len = start_symbol.len();
        let end_len = end_symbol.len();

        let mut start = 0;
        while let Some(open_idx) = value[start..].find(&start_symbol) {
            let abs_open = start + open_idx;
            if let Some(close_idx) = value[abs_open..].find(&end_symbol) {
                let abs_close = abs_open + close_idx;
                let expr = &value[abs_open + start_len..abs_close];
                let expr_trimmed = expr.trim();

                // 式の開始位置（{{ の後、トリム前の空白を考慮）- バイトオフセット
                let expr_start_byte_offset = abs_open + start_len + (expr.len() - expr.trim_start().len());

                // 式内での位置を計算（UTF-16対応）
                let before_expr = &value[..expr_start_byte_offset];
                let newline_count = before_expr.matches('\n').count();
                let expr_line = value_start_line + newline_count;

                let expr_col = if newline_count == 0 {
                    value_start_col + self.byte_offset_to_utf16_offset(before_expr, before_expr.len()) as u32
                } else {
                    let last_newline_pos = before_expr.rfind('\n').unwrap();
                    let after_newline = &before_expr[last_newline_pos + 1..];
                    self.byte_offset_to_utf16_offset(after_newline, after_newline.len()) as u32
                };

                // フィルタを除去
                let expr_to_check = expr_trimmed.split('|').next().unwrap_or(expr_trimmed);

                self.check_and_register_local_var_references_utf16(
                    expr_to_check,
                    uri,
                    expr_line as u32,
                    expr_col,
                    active_scopes,
                );

                start = abs_close + end_len;
            } else {
                break;
            }
        }
    }

    /// interpolation内からローカル変数参照を抽出（UTF-16対応版）
    fn extract_local_variable_references_from_interpolation_utf16(
        &self,
        text: &str,
        node: Node,
        source: &str,
        uri: &Url,
        active_scopes: &HashMap<String, (u32, u32)>,
    ) {
        let (start_symbol, end_symbol) = self.get_interpolate_symbols();
        let start_len = start_symbol.len();
        let end_len = end_symbol.len();

        let node_start_line = node.start_position().row as usize;
        let node_start_byte_col = node.start_position().column;
        let node_start_col = self.byte_col_to_utf16_col(source, node_start_line, node_start_byte_col);

        let mut start = 0;
        while let Some(open_idx) = text[start..].find(&start_symbol) {
            let abs_open = start + open_idx;
            if let Some(close_idx) = text[abs_open..].find(&end_symbol) {
                let abs_close = abs_open + close_idx;
                let expr = &text[abs_open + start_len..abs_close];
                let expr_trimmed = expr.trim();

                // 式の開始位置 - バイトオフセット
                let expr_start_byte_offset = abs_open + start_len + (expr.len() - expr.trim_start().len());

                // 式内での位置を計算（UTF-16対応）
                let before_expr = &text[..expr_start_byte_offset];
                let newline_count = before_expr.matches('\n').count();
                let expr_line = node_start_line + newline_count;

                let expr_col = if newline_count == 0 {
                    let utf16_offset = self.byte_offset_to_utf16_offset(text, expr_start_byte_offset);
                    node_start_col + utf16_offset as u32
                } else {
                    let last_newline_pos = before_expr.rfind('\n').unwrap();
                    let after_newline = &text[last_newline_pos + 1..expr_start_byte_offset];
                    self.byte_offset_to_utf16_offset(after_newline, after_newline.len()) as u32
                };

                // フィルタを除去
                let expr_to_check = expr_trimmed.split('|').next().unwrap_or(expr_trimmed);

                self.check_and_register_local_var_references_utf16(
                    expr_to_check,
                    uri,
                    expr_line as u32,
                    expr_col,
                    active_scopes,
                );

                start = abs_close + end_len;
            } else {
                break;
            }
        }
    }

    /// 式内のローカル変数参照をチェックして登録（UTF-16対応版）
    fn check_and_register_local_var_references_utf16(
        &self,
        expr: &str,
        uri: &Url,
        base_line: u32,
        base_col: u32, // UTF-16
        active_scopes: &HashMap<String, (u32, u32)>,
    ) {
        // 有効なローカル変数の名前だけをチェック
        for (var_name, _) in active_scopes {
            let positions = self.find_identifier_positions(expr, var_name);

            for (byte_offset, byte_len) in positions {
                // バイトオフセットからUTF-16位置を計算
                let before_identifier = &expr[..byte_offset];
                let identifier_text = &expr[byte_offset..byte_offset + byte_len];
                let newline_count = before_identifier.matches('\n').count();

                let start_line = base_line + newline_count as u32;
                let start_col = if newline_count == 0 {
                    base_col + self.byte_offset_to_utf16_offset(before_identifier, before_identifier.len()) as u32
                } else {
                    let last_newline_pos = before_identifier.rfind('\n').unwrap();
                    let after_newline = &before_identifier[last_newline_pos + 1..];
                    self.byte_offset_to_utf16_offset(after_newline, after_newline.len()) as u32
                };
                let end_line = start_line;
                let end_col = start_col + identifier_text.chars().map(|c| c.len_utf16()).sum::<usize>() as u32;

                let reference = HtmlLocalVariableReference {
                    variable_name: var_name.clone(),
                    uri: uri.clone(),
                    start_line,
                    start_col,
                    end_line,
                    end_col,
                };
                self.index.add_html_local_variable_reference(reference);
            }
        }
    }
}
