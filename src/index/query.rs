use std::collections::HashSet;

use tower_lsp::lsp_types::Url;

use super::Index;
use crate::model::{
    HtmlFormBinding, HtmlLocalVariable, Span, Symbol, SymbolKind,
    SymbolReference,
};

impl Index {
    // ========== クロスストアクエリ ==========

    /// シンボル名に対応するHTML内の参照を取得
    pub fn get_html_references_for_symbol(&self, symbol_name: &str) -> Vec<SymbolReference> {
        // $rootScope 形式を試す
        if let Some((_, property_path)) = self.parse_root_scope_symbol_name(symbol_name) {
            return self.get_html_references_for_root_scope(&property_path, symbol_name);
        }

        let (controller_name, property_path) =
            if let Some(parsed) = self.parse_scope_symbol_name(symbol_name) {
                parsed
            } else if let Some(parsed) = self.parse_controller_method_name(symbol_name) {
                parsed
            } else {
                return Vec::new();
            };

        let mut references = Vec::new();

        for entry in self.html.iter_all_html_scope_references() {
            let uri = entry.key();
            let html_refs = entry.value();

            for html_ref in html_refs {
                let direct_match = html_ref.property_path == property_path;

                let alias_match = if html_ref.property_path.contains('.') {
                    let parts: Vec<&str> = html_ref.property_path.splitn(2, '.').collect();
                    if parts.len() == 2 {
                        let alias = parts[0];
                        let prop = parts[1];
                        if let Some(resolved_controller) =
                            self.resolve_controller_by_alias(uri, html_ref.start_line, alias)
                        {
                            resolved_controller == controller_name && prop == property_path
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };

                if !direct_match && !alias_match {
                    continue;
                }

                let controllers =
                    self.resolve_controllers_for_html(uri, html_ref.start_line);
                if controllers.contains(&controller_name.to_string()) || alias_match {
                    references.push(SymbolReference {
                        name: symbol_name.to_string(),
                        uri: uri.clone(),
                        span: Span::new(
                            html_ref.start_line,
                            html_ref.start_col,
                            html_ref.end_line,
                            html_ref.end_col,
                        ),
                    });
                }
            }
        }

        references
    }

    fn get_html_references_for_root_scope(
        &self,
        property_path: &str,
        symbol_name: &str,
    ) -> Vec<SymbolReference> {
        let mut references = Vec::new();

        for entry in self.html.iter_all_html_scope_references() {
            let uri = entry.key();
            let html_refs = entry.value();

            for html_ref in html_refs {
                if html_ref.property_path != property_path {
                    continue;
                }

                let controllers =
                    self.resolve_controllers_for_html(uri, html_ref.start_line);
                let has_scope_property = controllers.iter().any(|ctrl| {
                    let scope_symbol = format!("{}.$scope.{}", ctrl, property_path);
                    self.definitions.has_definition(&scope_symbol)
                });

                if has_scope_property {
                    continue;
                }

                references.push(SymbolReference {
                    name: symbol_name.to_string(),
                    uri: uri.clone(),
                    span: Span::new(
                        html_ref.start_line,
                        html_ref.start_col,
                        html_ref.end_line,
                        html_ref.end_col,
                    ),
                });
            }
        }

        references
    }

    /// JS参照とHTML参照を合わせて取得
    pub fn get_all_references(&self, name: &str) -> Vec<SymbolReference> {
        let mut refs = self.definitions.get_references(name);
        refs.extend(self.get_html_references_for_symbol(name));
        refs
    }

    /// スコープ変数がHTMLから参照されているかチェック
    pub fn is_scope_variable_referenced(&self, symbol_name: &str) -> bool {
        !self.get_html_references_for_symbol(symbol_name).is_empty()
    }

    /// `<input ng-model="X">` のような暗黙的 scope 定義が存在するかをチェックする。
    ///
    /// `ng-model` ディレクティブは AngularJS が \$scope にプロパティを書き込むので、
    /// controller 側で明示的に `$scope.X = ...` を書かなくても \$scope に X が
    /// 存在する。このメソッドはそれを検出し、診断の false positive を抑制する。
    ///
    /// マッチ条件:
    /// 1. 同じ URI 内に `ng-model` のターゲット があること
    /// 2. その ng-model ターゲットの property tail が `property` と一致すること
    ///    (例: `ng-model="vm.currentPage"` の tail "currentPage" は
    ///     `{{ currentPage }}` の property "currentPage" にマッチ)
    /// 3. ng-model の位置で resolve される controller のいずれかが
    ///    `controller_name` と一致すること
    ///
    /// **明示的な `$scope.X` 定義がある場合はそちらが優先**される (本メソッドは
    /// 明示的定義が見つからなかった時のフォールバックとして使う想定)。
    pub fn has_ng_model_implicit_def(
        &self,
        uri: &Url,
        controller_name: &str,
        property: &str,
    ) -> bool {
        for target in self.html.get_ng_model_targets_for_uri(uri) {
            let target_property = ng_model_target_tail(&target.property_path);
            if target_property != property {
                continue;
            }

            // ng-model 位置での resolved controllers を取得して、要求された
            // controller_name と一致するものがあるか確認
            let target_controllers = if let Some(alias) =
                ng_model_target_alias(&target.property_path)
            {
                if let Some(ctrl) =
                    self.resolve_controller_by_alias(uri, target.start_line, alias)
                {
                    vec![ctrl]
                } else {
                    // alias unresolved: バインドはアクティブな controller の \$scope への書き込みと解釈
                    self.resolve_controllers_for_html(uri, target.start_line)
                }
            } else {
                self.resolve_controllers_for_html(uri, target.start_line)
            };

            if target_controllers.iter().any(|c| c == controller_name) {
                return true;
            }
        }
        false
    }

    /// HTMLファイルに対応するコントローラー名を解決
    pub fn resolve_controller_for_html(&self, uri: &Url, line: u32) -> Option<String> {
        if let Some(controller) = self.controllers.get_html_controller_at(uri, line) {
            return Some(controller);
        }
        self.templates.get_controller_for_template(uri)
    }

    /// HTMLファイルに対応する全コントローラー名を解決（外側から内側への順）
    pub fn resolve_controllers_for_html(&self, uri: &Url, line: u32) -> Vec<String> {
        let mut controllers = Vec::new();

        // ng-include継承
        let inherited = self.templates.get_inherited_controllers_for_template(uri);
        controllers.extend(inherited);

        // ローカルng-controller
        let local_controllers = self.controllers.get_html_controllers_at(uri, line);
        controllers.extend(local_controllers);

        // テンプレートバインディング
        if let Some(controller) = self.templates.get_controller_for_template(uri) {
            if !controllers.contains(&controller) {
                controllers.push(controller);
            }
        }

        // コンポーネントテンプレート
        if controllers.is_empty() {
            if let Some(binding) = self.components.get_component_binding_for_template(uri) {
                if let Some(ref controller_name) = binding.controller_name {
                    controllers.push(controller_name.clone());
                }
            }
        }

        // 重複を除去（順序は保持）
        let mut seen = HashSet::new();
        controllers.retain(|c| seen.insert(c.clone()));

        controllers
    }

    /// aliasに対応するコントローラー名を解決（ng-controller + コンポーネントテンプレート）
    pub fn resolve_controller_by_alias(
        &self,
        uri: &Url,
        line: u32,
        alias: &str,
    ) -> Option<String> {
        if let Some(name) = self.controllers.resolve_controller_by_alias(uri, line, alias) {
            return Some(name);
        }
        self.components
            .resolve_component_controller_by_alias(uri, alias)
    }

    /// ローカル変数の定義を取得（現在のファイル + 継承）
    pub fn find_local_variable_definition(
        &self,
        uri: &Url,
        variable_name: &str,
        line: u32,
    ) -> Option<HtmlLocalVariable> {
        if let Some(var) = self.html.find_local_variable_definition(uri, variable_name, line) {
            return Some(var);
        }

        // 継承されたローカル変数をチェック
        let inherited = self.templates.get_inherited_local_variables_for_template(uri);
        inherited
            .into_iter()
            .find(|v| v.name == variable_name)
            .map(|v| HtmlLocalVariable {
                name: v.name,
                source: v.source,
                uri: v.uri,
                scope_start_line: v.scope_start_line,
                scope_end_line: v.scope_end_line,
                name_start_line: v.name_start_line,
                name_start_col: v.name_start_col,
                name_end_line: v.name_end_line,
                name_end_col: v.name_end_col,
            })
    }

    /// フォームバインディングの定義を取得（現在のファイル + 継承）
    pub fn find_form_binding_definition(
        &self,
        uri: &Url,
        form_name: &str,
        line: u32,
    ) -> Option<HtmlFormBinding> {
        if let Some(binding) = self.html.find_form_binding_definition(uri, form_name, line) {
            return Some(binding);
        }

        let inherited = self.templates.get_inherited_form_bindings_for_template(uri);
        inherited
            .into_iter()
            .find(|b| b.name == form_name)
            .map(|b| HtmlFormBinding {
                name: b.name,
                uri: b.uri,
                scope_start_line: b.scope_start_line,
                scope_end_line: b.scope_end_line,
                name_start_line: b.name_start_line,
                name_start_col: b.name_start_col,
                name_end_line: b.name_end_line,
                name_end_col: b.name_end_col,
            })
    }

    /// テンプレートパスからURIを解決
    pub fn resolve_template_uri(&self, template_path: &str) -> Option<Url> {
        use crate::util::normalize_template_path;
        let normalized_path = normalize_template_path(template_path);
        let suffix = format!("/{}", normalized_path);

        // controller scopeのURIから検索
        for uri in self.controllers.html_controller_scope_uris() {
            let path = uri.path();
            if path.ends_with(&suffix) || path.ends_with(&normalized_path) {
                return Some(uri);
            }
        }

        // 解析済みHTML URIから検索
        for uri in self.templates.analyzed_html_uris() {
            let path = uri.path();
            if path.ends_with(&suffix) || path.ends_with(&normalized_path) {
                return Some(uri);
            }
        }

        // テンプレートストアで検索（フォールバック）
        self.templates.resolve_template_uri(template_path)
    }

    /// コントローラー名からバインドされているHTMLテンプレートのパスを取得
    pub fn get_templates_for_controller(&self, controller_name: &str) -> Vec<String> {
        let mut templates = self.templates.get_templates_for_controller(controller_name);
        let html_templates = self
            .controllers
            .get_html_templates_for_controller(controller_name);
        for t in html_templates {
            if !templates.contains(&t) {
                templates.push(t);
            }
        }
        templates
    }

    /// ドキュメントシンボル一覧を取得
    pub fn get_document_symbols(&self, uri: &Url) -> Vec<Symbol> {
        let mut symbols = self.definitions.get_definitions_for_uri(uri);

        // HTMLコントローラースコープ
        for scope in self.controllers.get_all_html_controller_scopes(uri) {
            symbols.push(Symbol {
                name: scope.controller_name.clone(),
                kind: SymbolKind::Controller,
                uri: scope.uri.clone(),
                definition_span: Span::new(scope.start_line, 0, scope.end_line, 0),
                name_span: Span::new(
                    scope.start_line,
                    0,
                    scope.start_line,
                    scope.controller_name.len() as u32,
                ),
                docs: Some("ng-controller".to_string()),
                parameters: None,
            });
        }

        // HTMLスコープ参照
        for r in self.html.get_html_scope_references(uri) {
            symbols.push(Symbol {
                name: r.property_path.clone(),
                kind: SymbolKind::ScopeProperty,
                uri: r.uri.clone(),
                definition_span: Span::new(r.start_line, r.start_col, r.end_line, r.end_col),
                name_span: Span::new(r.start_line, r.start_col, r.end_line, r.end_col),
                docs: None,
                parameters: None,
            });
        }

        // HTMLディレクティブ参照
        for r in self.html.get_all_directive_references_for_uri(uri) {
            symbols.push(Symbol {
                name: r.directive_name.clone(),
                kind: SymbolKind::Directive,
                uri: r.uri.clone(),
                definition_span: Span::new(r.start_line, r.start_col, r.end_line, r.end_col),
                name_span: Span::new(r.start_line, r.start_col, r.end_line, r.end_col),
                docs: None,
                parameters: None,
            });
        }

        symbols.sort_by(|a, b| {
            a.definition_span
                .start_line
                .cmp(&b.definition_span.start_line)
                .then(
                    a.definition_span
                        .start_col
                        .cmp(&b.definition_span.start_col),
                )
        });

        symbols
    }

    // ========== シンボル名パーサー ==========

    /// スコープシンボル名をパース: "ControllerName.$scope.propertyPath" -> (ControllerName, propertyPath)
    pub fn parse_scope_symbol_name(&self, symbol_name: &str) -> Option<(String, String)> {
        let scope_marker = ".$scope.";
        let idx = symbol_name.find(scope_marker)?;
        let controller_name = &symbol_name[..idx];
        let property_path = &symbol_name[idx + scope_marker.len()..];
        Some((controller_name.to_string(), property_path.to_string()))
    }

    /// $rootScopeシンボル名をパース
    pub fn parse_root_scope_symbol_name(&self, symbol_name: &str) -> Option<(String, String)> {
        let marker = ".$rootScope.";
        let idx = symbol_name.find(marker)?;
        let module_name = &symbol_name[..idx];
        let property_path = &symbol_name[idx + marker.len()..];
        Some((module_name.to_string(), property_path.to_string()))
    }

    /// コントローラーメソッド名をパース: "ControllerName.methodName"
    pub fn parse_controller_method_name(&self, symbol_name: &str) -> Option<(String, String)> {
        if symbol_name.contains(".$scope.") || symbol_name.contains(".$rootScope.") {
            return None;
        }
        let idx = symbol_name.find('.')?;
        let controller_name = &symbol_name[..idx];
        let method_name = &symbol_name[idx + 1..];
        if method_name.is_empty() {
            return None;
        }
        Some((controller_name.to_string(), method_name.to_string()))
    }
}

/// `ng-model` ターゲットの property_path から末尾のプロパティ名を抜き出す。
/// 例:
/// - `"currentPage"` → `"currentPage"`
/// - `"vm.currentPage"` → `"currentPage"`
/// - `"user.profile.name"` → `"name"` (depth が 2 以上の場合は最深部)
fn ng_model_target_tail(property_path: &str) -> &str {
    match property_path.rfind('.') {
        Some(idx) => &property_path[idx + 1..],
        None => property_path,
    }
}

/// `ng-model` ターゲットの property_path から先頭の alias 候補を返す。
/// `"."` を含まない場合は `None`。
fn ng_model_target_alias(property_path: &str) -> Option<&str> {
    property_path.find('.').map(|idx| &property_path[..idx])
}

#[cfg(test)]
mod ng_model_target_helpers_tests {
    use super::*;

    #[test]
    fn tail_returns_last_segment() {
        assert_eq!(ng_model_target_tail("currentPage"), "currentPage");
        assert_eq!(ng_model_target_tail("vm.currentPage"), "currentPage");
        assert_eq!(ng_model_target_tail("user.profile.name"), "name");
    }

    #[test]
    fn alias_returns_first_segment_or_none() {
        assert_eq!(ng_model_target_alias("currentPage"), None);
        assert_eq!(ng_model_target_alias("vm.currentPage"), Some("vm"));
        assert_eq!(ng_model_target_alias("user.profile.name"), Some("user"));
    }
}
