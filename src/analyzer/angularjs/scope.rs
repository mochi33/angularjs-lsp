use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::context::AnalyzerContext;
use super::AngularJsAnalyzer;
use crate::index::{Symbol, SymbolKind, SymbolReference};

impl AngularJsAnalyzer {
    /// $scope.property への代入を解析し、定義として登録する
    ///
    /// 認識パターン:
    /// ```javascript
    /// $scope.users = [];
    /// $scope.loadUsers = function() { ... };
    /// ```
    ///
    /// 一番最初の代入のみを定義として登録する
    /// 右辺が関数の場合は ScopeMethod、それ以外は ScopeProperty として登録
    pub(super) fn analyze_scope_assignment(&self, node: Node, source: &str, uri: &Url, ctx: &mut AnalyzerContext) {
        // $scope.xxx = ... パターンを検出
        if let Some(left) = node.child_by_field_name("left") {
            if left.kind() == "member_expression" {
                if let Some(object) = left.child_by_field_name("object") {
                    let obj_name = self.node_text(object, source);
                    if obj_name == "$scope" {
                        // スコープ情報を取得（コントローラー名と$scopeのDI状態を同時に取得）
                        let current_line = node.start_position().row as u32;
                        let (controller_name, has_scope) = match ctx.get_scope_info_at(current_line) {
                            Some((name, has_scope)) => (name, has_scope),
                            None => return, // スコープが見つからない場合はスキップ
                        };

                        // $scope がDIされていない場合はスキップ
                        if !has_scope {
                            return;
                        }

                        if let Some(property) = left.child_by_field_name("property") {
                            let prop_name = self.node_text(property, source);

                            // シンボル名を生成（コントローラー名.$scope.プロパティ名）
                            let full_name = format!("{}.$scope.{}", controller_name, prop_name);

                            // 既に定義済みの場合は参照として登録
                            if ctx.defined_scope_properties.contains_key(&full_name) {
                                // 代入の左辺も参照としてカウント
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
                                return;
                            }
                            ctx.defined_scope_properties.insert(full_name.clone(), true);

                            let prop_start = property.start_position();
                            let prop_end = property.end_position();

                            // JSDocを探す
                            let docs = self.extract_jsdoc_for_line(node.start_position().row, source);

                            // 右辺が関数かどうかを判定
                            let is_function = if let Some(right) = node.child_by_field_name("right") {
                                matches!(right.kind(), "function_expression" | "arrow_function")
                            } else {
                                false
                            };

                            let kind = if is_function {
                                SymbolKind::ScopeMethod
                            } else {
                                SymbolKind::ScopeProperty
                            };

                            let symbol = Symbol {
                                name: full_name,
                                kind,
                                uri: uri.clone(),
                                // 定義位置はプロパティ名の位置
                                start_line: self.offset_line(prop_start.row as u32),
                                start_col: prop_start.column as u32,
                                end_line: self.offset_line(prop_end.row as u32),
                                end_col: prop_end.column as u32,
                                // シンボル名の位置も同じ
                                name_start_line: self.offset_line(prop_start.row as u32),
                                name_start_col: prop_start.column as u32,
                                name_end_line: self.offset_line(prop_end.row as u32),
                                name_end_col: prop_end.column as u32,
                                docs,
                            };

                            self.index.add_definition(symbol);
                        }
                    }
                }
            }
        }
    }

    /// $scope.property への参照を解析し、参照として登録する
    ///
    /// 認識パターン:
    /// ```javascript
    /// $scope.users
    /// $scope.loadUsers()
    /// ```
    ///
    /// 代入の左辺（定義箇所）は別途 analyze_scope_assignment で処理されるため、
    /// ここでは代入の左辺以外の参照を登録する
    /// 定義がなくても参照として登録する（非同期処理内での定義など）
    pub(super) fn analyze_scope_member_access(&self, node: Node, source: &str, uri: &Url, ctx: &AnalyzerContext) {
        if let Some(object) = node.child_by_field_name("object") {
            let obj_name = self.node_text(object, source);
            if obj_name == "$scope" {
                // スコープ情報を取得（コントローラー名と$scopeのDI状態を同時に取得）
                let current_line = node.start_position().row as u32;
                let (controller_name, has_scope) = match ctx.get_scope_info_at(current_line) {
                    Some((name, has_scope)) => (name, has_scope),
                    None => return, // スコープが見つからない場合はスキップ
                };

                // $scope がDIされていない場合はスキップ
                if !has_scope {
                    return;
                }

                // 代入の左辺の場合はスキップ（定義として処理される）
                if let Some(parent) = node.parent() {
                    if parent.kind() == "assignment_expression" {
                        if let Some(left) = parent.child_by_field_name("left") {
                            if left.id() == node.id() {
                                return;
                            }
                        }
                    }
                }

                if let Some(property) = node.child_by_field_name("property") {
                    let prop_name = self.node_text(property, source);

                    // シンボル名を生成
                    let full_name = format!("{}.$scope.{}", controller_name, prop_name);

                    let start = property.start_position();
                    let end = property.end_position();

                    // 定義がなくても参照として登録（非同期処理内での定義など）
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
