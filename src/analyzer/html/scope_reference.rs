//! $scope参照の収集

use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::directives::{is_directive_attribute, is_literal_value_directive};
use crate::model::{HtmlNgModelTarget, HtmlScopeReference, HtmlUiSrefReference, Span, SymbolReference};

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
        // 要素のタグ名を取得 (component bindings 判定で必要)
        let element_tag_name = self
            .find_child_by_kind(start_tag, "tag_name")
            .map(|n| self.node_text(n, source));

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

                        // ui-router の `ui-sref="state[(...args)]"` は
                        // ディレクティブとしての扱いとは別に state 名参照として登録する。
                        // (`ui-sref-active` / `ui-sref-active-eq` は CSS class なので除外)
                        if matches!(attr_name.as_str(), "ui-sref" | "data-ui-sref") {
                            self.register_ui_sref_reference(
                                uri,
                                value,
                                value_start_line as u32,
                                value_start_col,
                            );
                        }

                        if is_directive_attribute(
                            &attr_name,
                            element_tag_name.as_deref(),
                            &self.index,
                        ) && !is_literal_value_directive(&attr_name)
                        {
                            // ngディレクティブ または custom directive / component binding:
                            // 属性値全体をAngular式として解析
                            // (ただし `ng-message` / `ng-messages-include` のような
                            //  リテラル文字列扱いのディレクティブは除外)
                            let property_paths = self.parse_angular_expression(value, &attr_name);
                            self.register_scope_references(uri, value, &property_paths, value_start_line as u32, value_start_col);

                            // ng-model="X" の値は \$scope への書き込みを行うので、
                            // 暗黙的な scope property 定義として記録する
                            // (controller 側で `$scope.X = ...` を書かなくても診断で
                            //  「未定義」と判定されないようにするため)
                            if attr_name == "ng-model" || attr_name == "data-ng-model" {
                                let target = HtmlNgModelTarget {
                                    property_path: value.to_string(),
                                    uri: uri.clone(),
                                    start_line: value_start_line as u32,
                                    start_col: value_start_col,
                                    end_line: value_start_line as u32,
                                    end_col: value_start_col
                                        + value.chars().map(|c| c.len_utf16()).sum::<usize>() as u32,
                                };
                                self.index.html.add_ng_model_target(target);
                            }
                        } else {
                            // 非ディレクティブ属性 または リテラル値ディレクティブ:
                            // インターポレーションのみを抽出 (例: `ng-message="{{key}}"` のように
                            //   稀にインターポレーションが含まれる可能性に備える)
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
                let identifier_text = &value[byte_offset..byte_offset + byte_len];
                let (start_line, start_col) =
                    self.position_in_text(value, byte_offset, value_start_line, value_start_col);
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
                self.index.html.add_html_scope_reference(html_reference);
            }
        }
    }

    /// `ui-sref="state-name[(...args)]"` の state 名部分を `HtmlUiSrefReference` と
    /// `SymbolReference` の両方として登録する。
    ///
    /// 値の形式:
    /// - `home` → state 名 = "home"
    /// - `home.detail` → state 名 = "home.detail" (dot-notation 子 state)
    /// - `home({id: 1})` → state 名 = "home" (引数部分は無視)
    ///
    /// 以下はスキップ:
    /// - 空文字列
    /// - 相対参照 (`.`, `^`, `^.foo` など) — state 名解決に親 state コンテキストが必要
    fn register_ui_sref_reference(
        &self,
        uri: &Url,
        value: &str,
        value_start_line: u32,
        value_start_col: u32,
    ) {
        // 引数部分 `(...)` の前までを state 名として切り出す
        let name_part = value.split('(').next().unwrap_or(value);
        let trimmed = name_part.trim();
        if trimmed.is_empty() {
            return;
        }

        // ui-router の相対参照は今は解決対象外
        if trimmed == "." || trimmed.starts_with('^') {
            return;
        }

        // 先頭の空白文字数だけ start_col をずらす
        let leading_whitespace_bytes = name_part.len() - name_part.trim_start().len();
        let utf16_leading = self.byte_offset_to_utf16_offset(name_part, leading_whitespace_bytes) as u32;
        let start_col = value_start_col + utf16_leading;
        let len_utf16 = trimmed.chars().map(|c| c.len_utf16()).sum::<usize>() as u32;
        let end_col = start_col + len_utf16;

        let reference = HtmlUiSrefReference {
            state_name: trimmed.to_string(),
            uri: uri.clone(),
            start_line: value_start_line,
            start_col,
            end_line: value_start_line,
            end_col,
        };
        self.index.html.add_ui_sref_reference(reference);

        // 既存の find-references インフラ用に SymbolReference も登録する
        // (これにより state 定義からの find-references で HTML 側の使用箇所も列挙される)
        self.index.definitions.add_reference(SymbolReference {
            name: trimmed.to_string(),
            uri: uri.clone(),
            span: Span::new(value_start_line, start_col, value_start_line, end_col),
        });
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

                // 式の開始位置を外側 (属性値) 座標系に変換
                let (expr_line, expr_col) = self.position_in_text(
                    value,
                    expr_start_byte_offset,
                    value_start_line as u32,
                    value_start_col,
                );

                // 式内でのプロパティパスの位置を登録
                for property_path in &property_paths {
                    // ローカル変数の場合はスキップ
                    let base_name = property_path.split('.').next().unwrap_or(property_path);
                    if self.index.find_local_variable_definition(uri, base_name, expr_line).is_some() {
                        continue;
                    }

                    // alias.property形式の場合、aliasまたはフォームバインディングが有効かチェック
                    if property_path.contains('.') {
                        let parts: Vec<&str> = property_path.splitn(2, '.').collect();
                        if parts.len() == 2 {
                            let potential_alias = parts[0];
                            let is_alias = self.index.resolve_controller_by_alias(uri, expr_line, potential_alias).is_some();
                            let is_form_binding = self.index.find_form_binding_definition(uri, potential_alias, expr_line).is_some();
                            if !is_alias && !is_form_binding {
                                continue;
                            }
                        }
                    }

                    // 式内で識別子のすべての出現位置を検索
                    let positions = self.find_identifier_positions(expr_trimmed, property_path);

                    for (byte_offset, byte_len) in positions {
                        let identifier_text = &expr_trimmed[byte_offset..byte_offset + byte_len];
                        let (start_line, start_col) =
                            self.position_in_text(expr_trimmed, byte_offset, expr_line, expr_col);
                        let end_line = start_line;
                        let end_col = start_col + identifier_text.chars().map(|c| c.len_utf16()).sum::<usize>() as u32;

                        let html_reference = HtmlScopeReference {
                            property_path: property_path.clone(),
                            uri: uri.clone(),
                            start_line,
                            start_col,
                            end_line,
                            end_col,
                        };
                        self.index.html.add_html_scope_reference(html_reference);
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
        byte_offset_to_utf16_offset(text, byte_offset)
    }

    /// 多行文字列 `text` 内のバイトオフセット `byte_offset` を、外側ソース座標系での
    /// `(line, utf16_col)` に変換する。
    ///
    /// `(base_line, base_col)` は `text` の先頭が外側ソースのどこにあるかを示す
    /// (`base_line`: 0-origin 行, `base_col`: UTF-16 列)。
    ///
    /// - `byte_offset` が `text` の最初の行 (text 内に改行がない範囲) にある場合:
    ///   `col = base_col + (text[..byte_offset] の UTF-16 文字数)`
    /// - 複数行に跨る場合: `byte_offset` が含まれる行の先頭 (= 外側ソースの行頭、
    ///   col 0) からの UTF-16 オフセットを `col` として返す。`base_col` は加算しない。
    ///
    /// この「複数行のときに `base_col` を加算しない」挙動は意図したもので、`text` が
    /// 行 0 では `base_col` から始まるが行 1 以降は外側行の先頭 (col 0) から始まる
    /// ためである。
    pub(super) fn position_in_text(
        &self,
        text: &str,
        byte_offset: usize,
        base_line: u32,
        base_col: u32,
    ) -> (u32, u32) {
        position_in_text(text, byte_offset, base_line, base_col)
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

                // 式の開始位置を外側 (text node) 座標系に変換
                let (expr_line, expr_col) = self.position_in_text(
                    text,
                    expr_start_byte_offset,
                    node_start_line as u32,
                    node_start_col,
                );

                for property_path in property_paths {
                    // ローカル変数の場合はスキップ
                    let base_name = property_path.split('.').next().unwrap_or(&property_path);
                    if self.index.find_local_variable_definition(uri, base_name, expr_line).is_some() {
                        continue;
                    }

                    // alias.property形式の場合、aliasまたはフォームバインディングが有効かチェック
                    // どちらでもない場合はスキップ（単純な識別子だけを登録）
                    if property_path.contains('.') {
                        let parts: Vec<&str> = property_path.splitn(2, '.').collect();
                        if parts.len() == 2 {
                            let potential_alias = parts[0];
                            // このaliasが有効かチェック
                            let is_alias = self.index.resolve_controller_by_alias(uri, expr_line, potential_alias).is_some();
                            // フォームバインディングかチェック
                            let is_form_binding = self.index.find_form_binding_definition(uri, potential_alias, expr_line).is_some();
                            if !is_alias && !is_form_binding {
                                // aliasでもフォームバインディングでもないのでスキップ
                                continue;
                            }
                        }
                    }

                    // 式内で識別子のすべての出現位置を検索
                    let positions = self.find_identifier_positions(expr_trimmed, &property_path);

                    for (byte_offset, byte_len) in positions {
                        let identifier_text = &expr_trimmed[byte_offset..byte_offset + byte_len];
                        let (start_line, start_col) =
                            self.position_in_text(expr_trimmed, byte_offset, expr_line, expr_col);
                        let end_line = start_line;
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
                        self.index.html.add_html_scope_reference(html_reference);
                    }
                }

                start = abs_close + end_len;
            } else {
                break;
            }
        }
    }
}

/// テキスト内でのバイトオフセットから UTF-16 コードユニット数を計算 (純粋関数版)。
///
/// メソッド版 `HtmlAngularJsAnalyzer::byte_offset_to_utf16_offset` の実装本体。
/// 内部の [`position_in_text`] からも参照される。
pub(super) fn byte_offset_to_utf16_offset(text: &str, byte_offset: usize) -> usize {
    let before = &text[..byte_offset.min(text.len())];
    before.chars().map(|c| c.len_utf16()).sum()
}

/// 多行文字列 `text` 内のバイトオフセット `byte_offset` を、外側ソース座標系での
/// `(line, utf16_col)` に変換する純粋関数。
///
/// 詳細は [`HtmlAngularJsAnalyzer::position_in_text`] を参照。
pub(super) fn position_in_text(
    text: &str,
    byte_offset: usize,
    base_line: u32,
    base_col: u32,
) -> (u32, u32) {
    let before = &text[..byte_offset.min(text.len())];
    let newline_count = before.matches('\n').count();
    let line = base_line + newline_count as u32;
    let col = if newline_count == 0 {
        // text の最初の行: 外側座標系では `base_col` 起点
        base_col + byte_offset_to_utf16_offset(before, before.len()) as u32
    } else {
        // text 内で改行を跨ぐ位置: 外側ソースでも別の行の頭 (col 0) 起点に揃う
        // ため、base_col は加算しない (これが意図した挙動)
        let last_newline_pos = before.rfind('\n').unwrap();
        let after_newline = &before[last_newline_pos + 1..];
        byte_offset_to_utf16_offset(after_newline, after_newline.len()) as u32
    };
    (line, col)
}

#[cfg(test)]
mod position_in_text_tests {
    use super::{byte_offset_to_utf16_offset, position_in_text};

    #[test]
    fn single_line_ascii() {
        // "hello world" の 'w' (byte 6) は、base=(10, 5) 起点なら (10, 5+6)
        let (line, col) = position_in_text("hello world", 6, 10, 5);
        assert_eq!((line, col), (10, 11));
    }

    #[test]
    fn single_line_with_supplementary_planes() {
        // 絵文字 "🎉" は UTF-8 4byte / UTF-16 2unit。byte_offset=4 (絵文字の直後) は
        // UTF-16 で +2 進んでいる。base=(0, 3) なら (0, 3+2)=(0, 5)。
        let (line, col) = position_in_text("🎉abc", 4, 0, 3);
        assert_eq!((line, col), (0, 5));
    }

    #[test]
    fn multi_line_starts_at_outer_column_zero() {
        // text="ab\ncd", byte_offset=4 ('d' の位置)。
        // 改行を跨ぐので line=base_line+1, col=外側行の先頭からの UTF-16 オフセット=1。
        // base_col=10 は加算してはならない (これが「multi-line で base_col を加算しない」挙動)。
        let (line, col) = position_in_text("ab\ncd", 4, 7, 10);
        assert_eq!((line, col), (8, 1));
    }

    #[test]
    fn multi_line_with_unicode_after_newline() {
        // "ab\n🎉xy", byte_offset=8 ('y') は改行後 UTF-8 5byte、UTF-16 3unit。
        // line=base_line+1, col=3 (改行後の文字列 "🎉x" の UTF-16 長)。
        let (line, col) = position_in_text("ab\n🎉xy", 8, 0, 99);
        assert_eq!((line, col), (1, 3));
    }

    #[test]
    fn at_byte_zero() {
        let (line, col) = position_in_text("hello", 0, 5, 7);
        assert_eq!((line, col), (5, 7));
    }

    #[test]
    fn at_text_end() {
        let (line, col) = position_in_text("abc", 3, 0, 0);
        assert_eq!((line, col), (0, 3));
    }

    #[test]
    fn byte_offset_helper_counts_utf16_units() {
        assert_eq!(byte_offset_to_utf16_offset("abc", 3), 3);
        assert_eq!(byte_offset_to_utf16_offset("a🎉b", 5), 3); // 'a' + '🎉' = 1 + 2 utf16
        assert_eq!(byte_offset_to_utf16_offset("", 0), 0);
        // byte_offset > len は飽和して全長を返す
        assert_eq!(byte_offset_to_utf16_offset("ab", 99), 2);
    }
}
