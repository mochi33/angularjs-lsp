use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::context::AnalyzerContext;
use super::utils::is_common_keyword;
use super::AngularJsAnalyzer;
use crate::index::SymbolReference;

impl AngularJsAnalyzer {
    /// メソッド呼び出しを解析し、参照として登録する
    ///
    /// 認識パターン:
    /// ```javascript
    /// UserService.getAll()
    /// AuthService.login(credentials)
    /// ```
    ///
    /// `$xxx`, `this`, `console` はスキップ
    /// DIされていないサービスへのアクセスは参照として登録しない
    pub(super) fn analyze_method_call(&self, node: Node, source: &str, uri: &Url, ctx: &AnalyzerContext) {
        if let Some(callee) = node.child_by_field_name("function") {
            if callee.kind() == "member_expression" {
                if let Some(object) = callee.child_by_field_name("object") {
                    if let Some(property) = callee.child_by_field_name("property") {
                        let obj_name = self.node_text(object, source);
                        let method_name = self.node_text(property, source);

                        if obj_name.starts_with('$') || obj_name == "this" || obj_name == "console" {
                            return;
                        }

                        // DIチェック: このスコープでサービスがDIされているか確認
                        let current_line = node.start_position().row as u32;
                        if !ctx.is_injected_at(&obj_name, current_line) {
                            return;
                        }

                        let full_name = format!("{}.{}", obj_name, method_name);

                        if self.index.has_definition(&full_name) {
                            let start = property.start_position();
                            let end = property.end_position();

                            let reference = SymbolReference {
                                name: full_name,
                                uri: uri.clone(),
                                start_line: self.offset_line(start.row as u32),
                                start_col: start.column as u32,
                                end_line: self.offset_line(end.row as u32),
                                end_col: end.column as u32,
                            };

                            self.index.add_reference(reference);
                        }
                    }
                }
            }
        }
    }

    /// メンバーアクセス（呼び出し以外）を解析し、参照として登録する
    ///
    /// 認識パターン:
    /// ```javascript
    /// var fn = UserService.getAll;  // 関数参照
    /// callback(AuthService.login);   // コールバック渡し
    /// ```
    ///
    /// DIされていないサービスへのアクセスは参照として登録しない
    pub(super) fn analyze_member_access(&self, node: Node, source: &str, uri: &Url, ctx: &AnalyzerContext) {
        if let Some(object) = node.child_by_field_name("object") {
            if let Some(property) = node.child_by_field_name("property") {
                let obj_name = self.node_text(object, source);
                let prop_name = self.node_text(property, source);

                if obj_name.starts_with('$') || obj_name == "this" || obj_name == "console" {
                    return;
                }

                // DIチェック: このスコープでサービスがDIされているか確認
                let current_line = node.start_position().row as u32;
                if !ctx.is_injected_at(&obj_name, current_line) {
                    return;
                }

                let full_name = format!("{}.{}", obj_name, prop_name);

                if self.index.has_definition(&full_name) {
                    let start = property.start_position();
                    let end = property.end_position();

                    let reference = SymbolReference {
                        name: full_name,
                        uri: uri.clone(),
                        start_line: self.offset_line(start.row as u32),
                        start_col: start.column as u32,
                        end_line: self.offset_line(end.row as u32),
                        end_col: end.column as u32,
                    };

                    self.index.add_reference(reference);
                }
            }
        }
    }

    /// 識別子を解析し、既知の定義への参照として登録する
    ///
    /// インデックスに存在するシンボル名と一致する識別子を参照として登録
    /// 短すぎる識別子やキーワードはスキップ
    /// DIされていないサービスへの参照は登録しない
    pub(super) fn analyze_identifier(&self, node: Node, source: &str, uri: &Url, ctx: &AnalyzerContext) {
        let name = self.node_text(node, source);

        if name.len() < 2 || is_common_keyword(&name) {
            return;
        }

        if self.index.has_definition(&name) {
            // DIチェック: このスコープでサービスがDIされているか確認
            let current_line = node.start_position().row as u32;
            if !ctx.is_injected_at(&name, current_line) {
                return;
            }

            let start = node.start_position();
            let end = node.end_position();

            let reference = SymbolReference {
                name,
                uri: uri.clone(),
                start_line: self.offset_line(start.row as u32),
                start_col: start.column as u32,
                end_line: self.offset_line(end.row as u32),
                end_col: end.column as u32,
            };

            self.index.add_reference(reference);
        }
    }
}
