use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::context::AnalyzerContext;
use super::AngularJsAnalyzer;
use crate::model::{Span, SymbolBuilder, SymbolKind, SymbolReference};

impl AngularJsAnalyzer {
    /// $scope から始まるメンバーアクセスチェーンのプロパティパスを再帰的に抽出する
    ///
    /// 例:
    /// - `$scope.user` → Some("user")
    /// - `$scope.user.name` → Some("user.name")
    /// - `$scope.user.name.first` → Some("user.name.first")
    /// - `other.prop` → None
    fn extract_scope_property_path(&self, node: Node, source: &str) -> Option<String> {
        if node.kind() != "member_expression" {
            return None;
        }

        let object = node.child_by_field_name("object")?;
        let property = node.child_by_field_name("property")?;
        let prop_name = self.node_text(property, source);

        if object.kind() == "identifier" {
            let obj_name = self.node_text(object, source);
            if obj_name == "$scope" {
                return Some(prop_name);
            }
            return None;
        }

        if object.kind() == "member_expression" {
            if let Some(parent_path) = self.extract_scope_property_path(object, source) {
                return Some(format!("{}.{}", parent_path, prop_name));
            }
        }

        None
    }

    /// $scope.property への代入を解析し、定義として登録する
    ///
    /// 認識パターン:
    /// ```javascript
    /// $scope.users = [];
    /// $scope.loadUsers = function() { ... };
    /// $scope.user.name = 'test';  // ネストされたプロパティ
    /// ```
    ///
    /// 一番最初の代入のみを定義として登録する
    /// 右辺が関数の場合は ScopeMethod、それ以外は ScopeProperty として登録
    pub(super) fn analyze_scope_assignment(&self, node: Node, source: &str, uri: &Url, ctx: &mut AnalyzerContext) {
        // $scope.xxx = ... または $scope.xxx.yyy = ... パターンを検出
        if let Some(left) = node.child_by_field_name("left") {
            if left.kind() == "member_expression" {
                if let Some(prop_path) = self.extract_scope_property_path(left, source) {
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
                        // シンボル名を生成（コントローラー名.$scope.プロパティパス）
                        let full_name = format!("{}.$scope.{}", controller_name, prop_path);

                        // 既に定義済みの場合は参照として登録
                        if ctx.defined_scope_properties.contains_key(&full_name) {
                            // 代入の左辺も参照としてカウント
                            let start = property.start_position();
                            let end = property.end_position();

                            let reference = SymbolReference {
                                name: full_name,
                                uri: uri.clone(),
                                span: Span::new(
                                    self.offset_line(start.row as u32),
                                    start.column as u32,
                                    self.offset_line(end.row as u32),
                                    end.column as u32,
                                ),
                            };

                            self.index.definitions.add_reference(reference);
                            return;
                        }
                        ctx.defined_scope_properties.insert(full_name.clone(), true);

                        let prop_start = property.start_position();
                        let prop_end = property.end_position();

                        // JSDocを探す
                        let docs = self.extract_jsdoc_for_line(node.start_position().row, source);

                        // 右辺が関数かどうかを判定し、パラメータを抽出
                        let (is_function, parameters) = if let Some(right) = node.child_by_field_name("right") {
                            let is_func = matches!(right.kind(), "function_expression" | "arrow_function");
                            let params = if is_func {
                                self.extract_function_params(right, source)
                            } else {
                                None
                            };
                            (is_func, params)
                        } else {
                            (false, None)
                        };

                        let kind = if is_function {
                            SymbolKind::ScopeMethod
                        } else {
                            SymbolKind::ScopeProperty
                        };

                        let def_span = Span::new(
                            self.offset_line(prop_start.row as u32),
                            prop_start.column as u32,
                            self.offset_line(prop_end.row as u32),
                            prop_end.column as u32,
                        );
                        let name_span = Span::new(
                            self.offset_line(prop_start.row as u32),
                            prop_start.column as u32,
                            self.offset_line(prop_end.row as u32),
                            prop_end.column as u32,
                        );

                        let mut builder = SymbolBuilder::new(full_name, kind, uri.clone())
                            .definition_span(def_span)
                            .name_span(name_span);

                        if let Some(docs_str) = docs {
                            builder = builder.docs(docs_str);
                        }
                        if let Some(params) = parameters {
                            builder = builder.parameters(params);
                        }

                        self.index.definitions.add_definition(builder.build());
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
    /// $scope.user.name  // ネストされたプロパティ参照
    /// ```
    ///
    /// 代入の左辺（定義箇所）は別途 analyze_scope_assignment で処理されるため、
    /// ここでは代入の左辺以外の参照を登録する
    /// 定義がなくても参照として登録する（非同期処理内での定義など）
    pub(super) fn analyze_scope_member_access(&self, node: Node, source: &str, uri: &Url, ctx: &AnalyzerContext) {
        if let Some(prop_path) = self.extract_scope_property_path(node, source) {
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
                // シンボル名を生成
                let full_name = format!("{}.$scope.{}", controller_name, prop_path);

                let start = property.start_position();
                let end = property.end_position();

                // 定義がなくても参照として登録（非同期処理内での定義など）
                let reference = SymbolReference {
                    name: full_name,
                    uri: uri.clone(),
                    span: Span::new(
                        self.offset_line(start.row as u32),
                        start.column as u32,
                        self.offset_line(end.row as u32),
                        end.column as u32,
                    ),
                };

                self.index.definitions.add_reference(reference);
            }
        }
    }

    /// $rootScope.property への代入を解析し、定義として登録する
    ///
    /// 認識パターン:
    /// ```javascript
    /// $rootScope.currentUser = {};
    /// $rootScope.logout = function() { ... };
    /// ```
    ///
    /// 一番最初の代入のみを定義として登録する
    /// 右辺が関数の場合は RootScopeMethod、それ以外は RootScopeProperty として登録
    pub(super) fn analyze_root_scope_assignment(&self, node: Node, source: &str, uri: &Url, ctx: &mut AnalyzerContext) {
        // $rootScope.xxx = ... パターンを検出
        if let Some(left) = node.child_by_field_name("left") {
            if left.kind() == "member_expression" {
                if let Some(object) = left.child_by_field_name("object") {
                    let obj_name = self.node_text(object, source);
                    if obj_name == "$rootScope" {
                        // スコープ情報を取得（モジュール名と$rootScopeのDI状態を同時に取得）
                        let current_line = node.start_position().row as u32;
                        let (module_name, has_root_scope) = match ctx.get_root_scope_info_at(current_line) {
                            Some((name, has_root_scope)) => (name, has_root_scope),
                            None => return, // モジュールが見つからない場合はスキップ
                        };

                        // $rootScope がDIされていない場合はスキップ
                        if !has_root_scope {
                            return;
                        }

                        if let Some(property) = left.child_by_field_name("property") {
                            let prop_name = self.node_text(property, source);

                            // シンボル名を生成（モジュール名.$rootScope.プロパティ名）
                            let full_name = format!("{}.$rootScope.{}", module_name, prop_name);

                            // 既に定義済みの場合は参照として登録
                            if ctx.defined_root_scope_properties.contains_key(&full_name) {
                                // 代入の左辺も参照としてカウント
                                let start = property.start_position();
                                let end = property.end_position();

                                let reference = SymbolReference {
                                    name: full_name,
                                    uri: uri.clone(),
                                    span: Span::new(
                                        self.offset_line(start.row as u32),
                                        start.column as u32,
                                        self.offset_line(end.row as u32),
                                        end.column as u32,
                                    ),
                                };

                                self.index.definitions.add_reference(reference);
                                return;
                            }
                            ctx.defined_root_scope_properties.insert(full_name.clone(), true);

                            let prop_start = property.start_position();
                            let prop_end = property.end_position();

                            // JSDocを探す
                            let docs = self.extract_jsdoc_for_line(node.start_position().row, source);

                            // 右辺が関数かどうかを判定し、パラメータを抽出
                            let (is_function, parameters) = if let Some(right) = node.child_by_field_name("right") {
                                let is_func = matches!(right.kind(), "function_expression" | "arrow_function");
                                let params = if is_func {
                                    self.extract_function_params(right, source)
                                } else {
                                    None
                                };
                                (is_func, params)
                            } else {
                                (false, None)
                            };

                            let kind = if is_function {
                                SymbolKind::RootScopeMethod
                            } else {
                                SymbolKind::RootScopeProperty
                            };

                            let def_span = Span::new(
                                self.offset_line(prop_start.row as u32),
                                prop_start.column as u32,
                                self.offset_line(prop_end.row as u32),
                                prop_end.column as u32,
                            );
                            let name_span = Span::new(
                                self.offset_line(prop_start.row as u32),
                                prop_start.column as u32,
                                self.offset_line(prop_end.row as u32),
                                prop_end.column as u32,
                            );

                            let mut builder = SymbolBuilder::new(full_name, kind, uri.clone())
                                .definition_span(def_span)
                                .name_span(name_span);

                            if let Some(docs_str) = docs {
                                builder = builder.docs(docs_str);
                            }
                            if let Some(params) = parameters {
                                builder = builder.parameters(params);
                            }

                            self.index.definitions.add_definition(builder.build());
                        }
                    }
                }
            }
        }
    }

    /// $rootScope.property への参照を解析し、参照として登録する
    ///
    /// 認識パターン:
    /// ```javascript
    /// $rootScope.currentUser
    /// $rootScope.logout()
    /// ```
    ///
    /// 代入の左辺（定義箇所）は別途 analyze_root_scope_assignment で処理されるため、
    /// ここでは代入の左辺以外の参照を登録する
    /// 定義がなくても参照として登録する（非同期処理内での定義など）
    pub(super) fn analyze_root_scope_member_access(&self, node: Node, source: &str, uri: &Url, ctx: &AnalyzerContext) {
        if let Some(object) = node.child_by_field_name("object") {
            let obj_name = self.node_text(object, source);
            if obj_name == "$rootScope" {
                // スコープ情報を取得（モジュール名と$rootScopeのDI状態を同時に取得）
                let current_line = node.start_position().row as u32;
                let (module_name, has_root_scope) = match ctx.get_root_scope_info_at(current_line) {
                    Some((name, has_root_scope)) => (name, has_root_scope),
                    None => return, // モジュールが見つからない場合はスキップ
                };

                // $rootScope がDIされていない場合はスキップ
                if !has_root_scope {
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
                    let full_name = format!("{}.$rootScope.{}", module_name, prop_name);

                    let start = property.start_position();
                    let end = property.end_position();

                    // 定義がなくても参照として登録（非同期処理内での定義など）
                    let reference = SymbolReference {
                        name: full_name,
                        uri: uri.clone(),
                        span: Span::new(
                            self.offset_line(start.row as u32),
                            start.column as u32,
                            self.offset_line(end.row as u32),
                            end.column as u32,
                        ),
                    };

                    self.index.definitions.add_reference(reference);
                }
            }
        }
    }
}
