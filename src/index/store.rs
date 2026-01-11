use dashmap::{DashMap, DashSet};
use tower_lsp::lsp_types::Url;
use tracing::debug;

use super::symbol::{Symbol, SymbolReference};

/// コントローラーのスコープ情報
#[derive(Clone, Debug)]
pub struct ControllerScope {
    pub name: String,
    pub uri: Url,
    pub start_line: u32,
    pub end_line: u32,
    /// DIで注入されているサービス名のリスト
    pub injected_services: Vec<String>,
}

/// テンプレートバインディングのソース
#[derive(Clone, Debug, PartialEq)]
pub enum BindingSource {
    NgController,
    RouteProvider,
    UibModal,
}

/// HTMLテンプレートとコントローラーのバインディング
#[derive(Clone, Debug)]
pub struct TemplateBinding {
    pub template_path: String,
    pub controller_name: String,
    pub source: BindingSource,
}

/// HTML内のng-controllerスコープ
#[derive(Clone, Debug)]
pub struct HtmlControllerScope {
    pub controller_name: String,
    /// "controller as alias"構文で指定されたalias名（例: "formCustomItem"）
    pub alias: Option<String>,
    pub uri: Url,
    pub start_line: u32,
    pub end_line: u32,
}

/// HTML内のスコープ参照
#[derive(Clone, Debug)]
pub struct HtmlScopeReference {
    pub property_path: String,
    pub uri: Url,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// ng-include経由で継承されるローカル変数
#[derive(Clone, Debug)]
pub struct InheritedLocalVariable {
    /// 変数名
    pub name: String,
    /// 変数の定義元ディレクティブ
    pub source: HtmlLocalVariableSource,
    /// 定義元のURI（親ファイル）
    pub uri: Url,
    /// 親ファイル内でのスコープ開始行
    pub scope_start_line: u32,
    /// 親ファイル内でのスコープ終了行
    pub scope_end_line: u32,
    /// 変数名の定義位置（親ファイル内）
    pub name_start_line: u32,
    pub name_start_col: u32,
    pub name_end_line: u32,
    pub name_end_col: u32,
}

/// ng-includeによる親子HTML関係
#[derive(Clone, Debug)]
pub struct NgIncludeBinding {
    pub parent_uri: Url,
    pub template_path: String,
    /// 親ファイルを起点として解決した絶対パス（ファイル名のみ）
    pub resolved_filename: String,
    /// ng-includeがある行
    pub line: u32,
    /// ng-includeがある位置での継承コントローラーリスト（外側から内側への順）
    pub inherited_controllers: Vec<String>,
    /// ng-includeがある位置での継承ローカル変数リスト
    pub inherited_local_variables: Vec<InheritedLocalVariable>,
    /// ng-includeがある位置での継承フォームバインディングリスト
    pub inherited_form_bindings: Vec<InheritedFormBinding>,
}

/// HTML内で定義されたローカル変数のソース
#[derive(Clone, Debug, PartialEq)]
pub enum HtmlLocalVariableSource {
    /// ng-init="counter = 0" -> "counter"
    NgInit,
    /// ng-repeat="item in items" -> "item"
    NgRepeatIterator,
    /// ng-repeat="(key, value) in obj" -> "key", "value"
    NgRepeatKeyValue,
}

impl HtmlLocalVariableSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            HtmlLocalVariableSource::NgInit => "ng-init",
            HtmlLocalVariableSource::NgRepeatIterator => "ng-repeat",
            HtmlLocalVariableSource::NgRepeatKeyValue => "ng-repeat",
        }
    }
}

/// HTML内で定義されたローカル変数（ng-init, ng-repeat由来）
#[derive(Clone, Debug)]
pub struct HtmlLocalVariable {
    /// 変数名（例: "item", "key", "value", "counter"）
    pub name: String,
    /// 変数の定義元ディレクティブ
    pub source: HtmlLocalVariableSource,
    /// 定義元のURI
    pub uri: Url,
    /// スコープの開始行（定義要素の開始）
    pub scope_start_line: u32,
    /// スコープの終了行（定義要素の終了）
    pub scope_end_line: u32,
    /// 変数名の定義位置（正確な位置）
    pub name_start_line: u32,
    pub name_start_col: u32,
    pub name_end_line: u32,
    pub name_end_col: u32,
}

/// HTML内のローカル変数への参照
#[derive(Clone, Debug)]
pub struct HtmlLocalVariableReference {
    /// 参照している変数名
    pub variable_name: String,
    /// 参照位置のURI
    pub uri: Url,
    /// 参照位置
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// HTML内の<form name="x">で定義されるフォームバインディング
/// AngularJSは自動的に$scope.xにフォームコントローラーをバインドする
#[derive(Clone, Debug)]
pub struct HtmlFormBinding {
    /// フォーム名（例: "userForm"）
    pub name: String,
    /// 定義元のURI
    pub uri: Url,
    /// スコープの開始行（ng-controllerスコープの開始、またはファイル先頭）
    /// フォームはformタグの範囲ではなく、コントローラースコープ全体で参照可能
    pub scope_start_line: u32,
    /// スコープの終了行（ng-controllerスコープの終了、またはファイル末尾）
    pub scope_end_line: u32,
    /// name属性値の位置（正確な位置）
    pub name_start_line: u32,
    pub name_start_col: u32,
    pub name_end_line: u32,
    pub name_end_col: u32,
}

/// ng-include経由で継承されるフォームバインディング
#[derive(Clone, Debug)]
pub struct InheritedFormBinding {
    /// フォーム名
    pub name: String,
    /// 定義元のURI（親ファイル）
    pub uri: Url,
    /// 親ファイル内でのスコープ開始行
    pub scope_start_line: u32,
    /// 親ファイル内でのスコープ終了行
    pub scope_end_line: u32,
    /// フォーム名の定義位置（親ファイル内）
    pub name_start_line: u32,
    pub name_start_col: u32,
    pub name_end_line: u32,
    pub name_end_col: u32,
}

pub struct SymbolIndex {
    definitions: DashMap<String, Vec<Symbol>>,
    references: DashMap<String, Vec<SymbolReference>>,
    document_symbols: DashMap<Url, Vec<String>>,
    /// コントローラーのスコープ情報（URI -> コントローラー名 -> スコープ）
    controller_scopes: DashMap<Url, Vec<ControllerScope>>,
    /// テンプレートバインディング（正規化されたtemplate_path -> binding）
    template_bindings: DashMap<String, TemplateBinding>,
    /// HTML内のng-controllerスコープ
    html_controller_scopes: DashMap<Url, Vec<HtmlControllerScope>>,
    /// HTML内のスコープ参照
    html_scope_references: DashMap<Url, Vec<HtmlScopeReference>>,
    /// ng-includeによる親子HTML関係（正規化されたtemplate_path -> binding）
    ng_include_bindings: DashMap<String, NgIncludeBinding>,
    /// HTML内のローカル変数定義（URI -> Vec<HtmlLocalVariable>）
    html_local_variables: DashMap<Url, Vec<HtmlLocalVariable>>,
    /// HTML内のローカル変数参照（変数名 -> Vec<HtmlLocalVariableReference>）
    html_local_variable_references: DashMap<String, Vec<HtmlLocalVariableReference>>,
    /// HTML内のフォームバインディング（URI -> Vec<HtmlFormBinding>）
    html_form_bindings: DashMap<Url, Vec<HtmlFormBinding>>,
    /// 再解析が必要なHTMLファイル（ng-include登録時に子HTMLが既に解析済みだった場合）
    pending_reanalysis: DashSet<Url>,
    /// 解析済みのHTMLファイルのURI（再解析判定用）
    analyzed_html_files: DashSet<Url>,
}

impl SymbolIndex {
    pub fn new() -> Self {
        Self {
            definitions: DashMap::new(),
            references: DashMap::new(),
            document_symbols: DashMap::new(),
            controller_scopes: DashMap::new(),
            template_bindings: DashMap::new(),
            html_controller_scopes: DashMap::new(),
            html_scope_references: DashMap::new(),
            ng_include_bindings: DashMap::new(),
            html_local_variables: DashMap::new(),
            html_local_variable_references: DashMap::new(),
            html_form_bindings: DashMap::new(),
            pending_reanalysis: DashSet::new(),
            analyzed_html_files: DashSet::new(),
        }
    }

    /// コントローラーのスコープ情報を追加
    pub fn add_controller_scope(&self, scope: ControllerScope) {
        let uri = scope.uri.clone();
        self.controller_scopes.entry(uri).or_default().push(scope);
    }

    /// 指定位置のコントローラー名を取得
    pub fn get_controller_at(&self, uri: &Url, line: u32) -> Option<String> {
        if let Some(scopes) = self.controller_scopes.get(uri) {
            for scope in scopes.iter() {
                if line >= scope.start_line && line <= scope.end_line {
                    return Some(scope.name.clone());
                }
            }
        }
        None
    }

    /// 指定位置のコントローラーでDIされているサービスを取得
    pub fn get_injected_services_at(&self, uri: &Url, line: u32) -> Vec<String> {
        if let Some(scopes) = self.controller_scopes.get(uri) {
            for scope in scopes.iter() {
                if line >= scope.start_line && line <= scope.end_line {
                    return scope.injected_services.clone();
                }
            }
        }
        Vec::new()
    }

    pub fn add_definition(&self, symbol: Symbol) {
        let name = symbol.name.clone();
        let uri = symbol.uri.clone();

        let mut entry = self.definitions.entry(name.clone()).or_default();
        // 重複チェック
        let is_duplicate = entry.iter().any(|s| {
            s.uri == symbol.uri
                && s.start_line == symbol.start_line
                && s.start_col == symbol.start_col
        });
        if !is_duplicate {
            entry.push(symbol);
            self.document_symbols.entry(uri).or_default().push(name);
        }
    }

    pub fn add_reference(&self, reference: SymbolReference) {
        let name = reference.name.clone();
        let uri = reference.uri.clone();

        let mut entry = self.references.entry(name.clone()).or_default();
        // 重複チェック
        let is_duplicate = entry.iter().any(|r| {
            r.uri == reference.uri
                && r.start_line == reference.start_line
                && r.start_col == reference.start_col
        });
        if !is_duplicate {
            entry.push(reference);
            self.document_symbols.entry(uri).or_default().push(name);
        }
    }

    pub fn get_definitions(&self, name: &str) -> Vec<Symbol> {
        self.definitions
            .get(name)
            .map(|v| v.value().clone())
            .unwrap_or_default()
    }

    /// 指定した名前がService/Factoryかどうかを判定
    pub fn is_service_or_factory(&self, name: &str) -> bool {
        if let Some(symbols) = self.definitions.get(name) {
            return symbols.iter().any(|s| {
                s.kind == super::symbol::SymbolKind::Service
                    || s.kind == super::symbol::SymbolKind::Factory
            });
        }
        false
    }

    pub fn get_references(&self, name: &str) -> Vec<SymbolReference> {
        self.references
            .get(name)
            .map(|v| v.value().clone())
            .unwrap_or_default()
    }

    /// シンボル名に対応するHTML内の参照を取得
    /// シンボル名の形式: "ControllerName.$scope.propertyPath" または "ControllerName.methodName" または "ModuleName.$rootScope.propertyPath"
    pub fn get_html_references_for_symbol(&self, symbol_name: &str) -> Vec<SymbolReference> {
        // $rootScope 形式を試す
        if let Some((_, property_path)) = self.parse_root_scope_symbol_name(symbol_name) {
            return self.get_html_references_for_root_scope(&property_path, symbol_name);
        }

        // シンボル名からコントローラー名とプロパティパスを抽出
        // まず $scope 形式を試す
        let (controller_name, property_path) = if let Some(parsed) = self.parse_scope_symbol_name(symbol_name) {
            parsed
        } else if let Some(parsed) = self.parse_controller_method_name(symbol_name) {
            // controller as 構文の this.method パターン (ControllerName.methodName)
            parsed
        } else {
            return Vec::new();
        };

        let mut references = Vec::new();

        // 全てのHTML参照を走査
        for entry in self.html_scope_references.iter() {
            let uri = entry.key();
            let html_refs = entry.value();

            for html_ref in html_refs {
                // 直接のプロパティパスが一致するか確認（通常の$scope参照）
                let direct_match = html_ref.property_path == property_path;

                // alias.property形式のマッチング（controller as alias構文）
                let alias_match = if html_ref.property_path.contains('.') {
                    // "alias.property"形式をパース
                    let parts: Vec<&str> = html_ref.property_path.splitn(2, '.').collect();
                    if parts.len() == 2 {
                        let alias = parts[0];
                        let prop = parts[1];
                        // aliasに対応するコントローラーを解決
                        if let Some(resolved_controller) = self.resolve_controller_by_alias(uri, html_ref.start_line, alias) {
                            // コントローラーとプロパティの両方が一致するか確認
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

                // このHTML参照がどのコントローラーに属するか確認
                let controllers = self.resolve_controllers_for_html(uri, html_ref.start_line);
                if controllers.contains(&controller_name.to_string()) || alias_match {
                    references.push(SymbolReference {
                        name: symbol_name.to_string(),
                        uri: uri.clone(),
                        start_line: html_ref.start_line,
                        start_col: html_ref.start_col,
                        end_line: html_ref.end_line,
                        end_col: html_ref.end_col,
                    });
                }
            }
        }

        references
    }

    /// $rootScopeプロパティに対応するHTML参照を取得
    /// $rootScopeはグローバルスコープなので、コントローラー制約なしでマッチング
    /// ただし、同名の$scopeプロパティがある場合は$scopeが優先（除外）
    fn get_html_references_for_root_scope(&self, property_path: &str, symbol_name: &str) -> Vec<SymbolReference> {
        let mut references = Vec::new();

        // 全てのHTML参照を走査
        for entry in self.html_scope_references.iter() {
            let uri = entry.key();
            let html_refs = entry.value();

            for html_ref in html_refs {
                // プロパティパスが一致するか確認
                if html_ref.property_path != property_path {
                    continue;
                }

                // 同名の$scopeプロパティがあるコントローラーでは$scopeが優先
                // そのHTML位置に対応するコントローラーの$scopeに同名プロパティがあればスキップ
                let controllers = self.resolve_controllers_for_html(uri, html_ref.start_line);
                let has_scope_property = controllers.iter().any(|ctrl| {
                    let scope_symbol = format!("{}.$scope.{}", ctrl, property_path);
                    self.has_definition(&scope_symbol)
                });

                if has_scope_property {
                    // $scopeプロパティが優先されるのでスキップ
                    continue;
                }

                references.push(SymbolReference {
                    name: symbol_name.to_string(),
                    uri: uri.clone(),
                    start_line: html_ref.start_line,
                    start_col: html_ref.start_col,
                    end_line: html_ref.end_line,
                    end_col: html_ref.end_col,
                });
            }
        }

        references
    }

    /// スコープシンボル名をパース: "ControllerName.$scope.propertyPath" -> (ControllerName, propertyPath)
    fn parse_scope_symbol_name(&self, symbol_name: &str) -> Option<(String, String)> {
        let scope_marker = ".$scope.";
        let idx = symbol_name.find(scope_marker)?;
        let controller_name = &symbol_name[..idx];
        let property_path = &symbol_name[idx + scope_marker.len()..];
        Some((controller_name.to_string(), property_path.to_string()))
    }

    /// $rootScopeシンボル名をパース: "ModuleName.$rootScope.propertyPath" -> (ModuleName, propertyPath)
    fn parse_root_scope_symbol_name(&self, symbol_name: &str) -> Option<(String, String)> {
        let marker = ".$rootScope.";
        let idx = symbol_name.find(marker)?;
        let module_name = &symbol_name[..idx];
        let property_path = &symbol_name[idx + marker.len()..];
        Some((module_name.to_string(), property_path.to_string()))
    }

    /// コントローラーメソッド名をパース: "ControllerName.methodName" -> (ControllerName, methodName)
    /// controller as 構文で this.method パターンを使用した場合
    fn parse_controller_method_name(&self, symbol_name: &str) -> Option<(String, String)> {
        // $scope または $rootScope を含まない場合のみ処理
        if symbol_name.contains(".$scope.") || symbol_name.contains(".$rootScope.") {
            return None;
        }
        // 最初の . で分割
        let idx = symbol_name.find('.')?;
        let controller_name = &symbol_name[..idx];
        let method_name = &symbol_name[idx + 1..];
        // method_name が空でなく、さらに . を含まない場合のみ有効
        // (ServiceName.methodName は OK、複雑なパスは除外)
        if method_name.is_empty() {
            return None;
        }
        Some((controller_name.to_string(), method_name.to_string()))
    }

    /// JS参照とHTML参照を合わせて取得
    pub fn get_all_references(&self, name: &str) -> Vec<SymbolReference> {
        let mut refs = self.get_references(name);
        refs.extend(self.get_html_references_for_symbol(name));
        refs
    }

    pub fn has_definition(&self, name: &str) -> bool {
        self.definitions.contains_key(name)
    }

    pub fn get_all_definitions(&self) -> Vec<Symbol> {
        self.definitions
            .iter()
            .flat_map(|entry| entry.value().clone())
            .collect()
    }

    /// 参照のみ存在するシンボル名を取得（定義がないもの）
    pub fn get_reference_only_names(&self) -> Vec<String> {
        self.references
            .iter()
            .filter(|entry| !self.definitions.contains_key(entry.key()))
            .map(|entry| entry.key().clone())
            .collect()
    }

    pub fn clear_document(&self, uri: &Url) {
        if let Some((_, symbols)) = self.document_symbols.remove(uri) {
            for symbol_name in symbols {
                if let Some(mut defs) = self.definitions.get_mut(&symbol_name) {
                    defs.retain(|s| &s.uri != uri);
                }
                if let Some(mut refs) = self.references.get_mut(&symbol_name) {
                    refs.retain(|r| &r.uri != uri);
                }
            }
        }
        // コントローラースコープもクリア
        self.controller_scopes.remove(uri);
        // HTMLスコープもクリア
        self.html_controller_scopes.remove(uri);
        self.html_scope_references.remove(uri);
        // このURIが親のng-includeバインディングをクリア
        self.clear_ng_include_bindings_for_parent(uri);
        // HTMLローカル変数もクリア
        self.html_local_variables.remove(uri);
        // このURIのローカル変数参照をクリア
        for mut entry in self.html_local_variable_references.iter_mut() {
            entry.value_mut().retain(|r| &r.uri != uri);
        }
        // HTMLフォームバインディングもクリア
        self.html_form_bindings.remove(uri);
    }

    pub fn remove_document(&self, uri: &Url) {
        self.clear_document(uri);
    }

    /// HTML参照情報のみをクリア（Pass 3で収集する情報）
    /// ng-controllerスコープ、ng-includeバインディング、フォームバインディングは保持
    pub fn clear_html_references(&self, uri: &Url) {
        // $scope参照をクリア
        self.html_scope_references.remove(uri);
        // ローカル変数定義をクリア
        self.html_local_variables.remove(uri);
        // このURIのローカル変数参照をクリア
        for mut entry in self.html_local_variable_references.iter_mut() {
            entry.value_mut().retain(|r| &r.uri != uri);
        }
    }

    /// 再解析が必要なURIを取得してキューをクリア
    pub fn take_pending_reanalysis(&self) -> Vec<Url> {
        let uris: Vec<Url> = self.pending_reanalysis.iter().map(|r| r.clone()).collect();
        self.pending_reanalysis.clear();
        uris
    }

    /// 指定URIを再解析キューから削除（自分自身の再解析を防ぐ）
    pub fn remove_from_pending_reanalysis(&self, uri: &Url) {
        self.pending_reanalysis.remove(uri);
    }

    /// HTMLファイルを解析済みとしてマーク
    pub fn mark_html_analyzed(&self, uri: &Url) {
        self.analyzed_html_files.insert(uri.clone());
    }

    // ========== テンプレートバインディング関連 ==========

    /// テンプレートバインディングを追加
    /// テンプレートが親として持っているng-include bindingの継承情報も更新する
    pub fn add_template_binding(&self, binding: TemplateBinding) {
        let normalized_path = Self::normalize_template_path(&binding.template_path);
        let controller_name = binding.controller_name.clone();

        // template_pathも正規化済みの値で保存
        let normalized_binding = TemplateBinding {
            template_path: normalized_path.clone(),
            controller_name: binding.controller_name,
            source: binding.source,
        };
        self.template_bindings.insert(normalized_path.clone(), normalized_binding);

        // このテンプレートが親として持っているng-include bindingの継承情報を更新
        // （$uibModalでバインドされたコントローラーをng-includeの子に伝播）
        // テンプレートバインディングはローカル変数やフォームバインディングを持たないので空
        self.propagate_inheritance_to_children(&normalized_path, &[controller_name], &[], &[]);
    }

    /// テンプレートパスを正規化（クエリパラメータを除去、`../`を除去）
    /// 相対パスの`../`部分は除去し、残りのパスを返す
    /// 例: "../foo/bar/baz.html" -> "foo/bar/baz.html"
    /// 例: "/foo/bar/baz.html" -> "foo/bar/baz.html"
    fn normalize_template_path(path: &str) -> String {
        // クエリパラメータを除去
        let path = path.split('?').next().unwrap_or(path);

        // 先頭の`../`や`./`や`/`を除去して、実際のパス部分を取得
        let mut normalized = path;
        while normalized.starts_with("../") {
            normalized = &normalized[3..];
        }
        while normalized.starts_with("./") {
            normalized = &normalized[2..];
        }
        if normalized.starts_with('/') {
            normalized = &normalized[1..];
        }

        normalized.to_string()
    }

    /// URIからコントローラー名を取得（テンプレートバインディング経由）
    pub fn get_controller_for_template(&self, uri: &Url) -> Option<String> {
        let path = uri.path();
        let filename = path.rsplit('/').next()?;

        // 方法1: 正規化パスでマッチング（パス末尾で比較）
        for entry in self.template_bindings.iter() {
            let normalized_path = entry.key();
            if path.ends_with(&format!("/{}", normalized_path)) || path == format!("/{}", normalized_path) {
                return Some(entry.value().controller_name.clone());
            }
        }

        // 方法2: ファイル名のみでマッチング（フォールバック）
        if let Some(binding) = self.template_bindings.get(filename) {
            return Some(binding.controller_name.clone());
        }
        None
    }

    /// URIからテンプレートバインディングのソース情報を取得
    /// 返り値: (コントローラー名, ソースタイプ, コントローラー定義のURI, 定義行)
    pub fn get_template_binding_source(&self, uri: &Url) -> Option<(String, BindingSource, Url, u32)> {
        let path = uri.path();
        let filename = path.rsplit('/').next()?;

        // テンプレートバインディングを検索
        let binding = {
            // 方法1: 正規化パスでマッチング
            let mut found = None;
            for entry in self.template_bindings.iter() {
                let normalized_path = entry.key();
                if path.ends_with(&format!("/{}", normalized_path)) || path == format!("/{}", normalized_path) {
                    found = Some(entry.value().clone());
                    break;
                }
            }
            // 方法2: ファイル名のみでマッチング
            if found.is_none() {
                found = self.template_bindings.get(filename).map(|b| b.clone());
            }
            found
        }?;

        // コントローラー定義からURIを取得
        let definitions = self.get_definitions(&binding.controller_name);
        if let Some(def) = definitions.first() {
            Some((binding.controller_name, binding.source, def.uri.clone(), def.start_line))
        } else {
            None
        }
    }

    /// コントローラー名からバインドされているHTMLテンプレートのパスを取得
    pub fn get_templates_for_controller(&self, controller_name: &str) -> Vec<String> {
        let mut templates = Vec::new();

        // 1. テンプレートバインディングから検索（$routeProvider, $uibModal）
        for entry in self.template_bindings.iter() {
            if entry.value().controller_name == controller_name {
                templates.push(entry.value().template_path.clone());
            }
        }

        // 2. HTML内のng-controllerスコープから検索
        for entry in self.html_controller_scopes.iter() {
            for scope in entry.value() {
                if scope.controller_name == controller_name {
                    // URIからパスを抽出
                    let path = entry.key().path().to_string();
                    if !templates.contains(&path) {
                        templates.push(path);
                    }
                }
            }
        }

        templates
    }

    // ========== ng-includeバインディング関連 ==========

    /// ng-includeバインディングを追加
    /// 子ファイルが親として持っているng-include bindingの継承情報も更新する
    pub fn add_ng_include_binding(&self, binding: NgIncludeBinding) {
        let normalized_path = Self::normalize_template_path(&binding.template_path);
        let resolved_filename = binding.resolved_filename.clone();
        let inherited_controllers = binding.inherited_controllers.clone();
        let inherited_local_variables = binding.inherited_local_variables.clone();
        let inherited_form_bindings = binding.inherited_form_bindings.clone();

        debug!(
            "add_ng_include_binding: {} (resolved: {}) -> {:?}",
            normalized_path, resolved_filename, inherited_controllers
        );
        self.ng_include_bindings.insert(normalized_path.clone(), binding);

        // 子HTMLが既に解析済みかチェックし、解析済みなら再解析キューに追加
        // これにより、親のフォームバインディングが子HTMLで参照可能になる
        self.queue_child_for_reanalysis(&resolved_filename, &normalized_path);

        // 子ファイル（normalized_path）が親として登録しているng-include bindingがあれば、
        // その継承情報を更新する（継承チェーンの伝播）
        self.propagate_inheritance_to_children(
            &normalized_path,
            &inherited_controllers,
            &inherited_local_variables,
            &inherited_form_bindings,
        );
    }

    /// 子HTMLが解析済みなら再解析キューに追加
    fn queue_child_for_reanalysis(&self, resolved_filename: &str, normalized_path: &str) {
        // 解析済みHTMLファイルを走査して、ファイル名が一致するものを探す
        for uri in self.analyzed_html_files.iter() {
            let uri_path = uri.path();
            // ファイル名またはパスが一致するかチェック
            if uri_path.ends_with(&format!("/{}", resolved_filename))
                || uri_path.ends_with(&format!("/{}", normalized_path))
                || uri_path == format!("/{}", resolved_filename)
                || uri_path == format!("/{}", normalized_path)
            {
                debug!(
                    "queue_child_for_reanalysis: {} needs reanalysis (matched by {})",
                    uri.key(), normalized_path
                );
                self.pending_reanalysis.insert(uri.clone());
            }
        }
    }

    /// 継承情報を子ファイルのng-include bindingに伝播させる
    fn propagate_inheritance_to_children(
        &self,
        child_path: &str,
        parent_controllers: &[String],
        parent_local_variables: &[InheritedLocalVariable],
        parent_form_bindings: &[InheritedFormBinding],
    ) {
        // child_pathをparent_uriとして持つng-include bindingを探す
        let mut updates: Vec<(
            String,
            Vec<String>,
            Vec<InheritedLocalVariable>,
            Vec<InheritedFormBinding>,
        )> = Vec::new();

        for entry in self.ng_include_bindings.iter() {
            let binding = entry.value();
            // parent_uriのパスがchild_pathで終わるかチェック
            let parent_uri_path = binding.parent_uri.path();
            if parent_uri_path.ends_with(&format!("/{}", child_path)) ||
               parent_uri_path.ends_with(child_path) {
                // 継承情報を更新（親のコントローラー + 現在のコントローラー）
                let mut new_controllers = parent_controllers.to_vec();
                for ctrl in &binding.inherited_controllers {
                    if !new_controllers.contains(ctrl) {
                        new_controllers.push(ctrl.clone());
                    }
                }

                // ローカル変数を更新（親のローカル変数 + 現在のローカル変数）
                let mut new_local_variables = parent_local_variables.to_vec();
                for var in &binding.inherited_local_variables {
                    if !new_local_variables.iter().any(|v| v.name == var.name) {
                        new_local_variables.push(var.clone());
                    }
                }

                // フォームバインディングを更新（親のフォームバインディング + 現在のフォームバインディング）
                let mut new_form_bindings = parent_form_bindings.to_vec();
                for form in &binding.inherited_form_bindings {
                    if !new_form_bindings.iter().any(|f| f.name == form.name) {
                        new_form_bindings.push(form.clone());
                    }
                }

                // いずれかが更新された場合のみ追加
                let controllers_changed = new_controllers != binding.inherited_controllers;
                let local_vars_changed = new_local_variables.len() != binding.inherited_local_variables.len()
                    || !new_local_variables.iter().all(|v| binding.inherited_local_variables.iter().any(|bv| bv.name == v.name));
                let forms_changed = new_form_bindings.len() != binding.inherited_form_bindings.len()
                    || !new_form_bindings.iter().all(|f| binding.inherited_form_bindings.iter().any(|bf| bf.name == f.name));

                if controllers_changed || local_vars_changed || forms_changed {
                    updates.push((
                        entry.key().clone(),
                        new_controllers,
                        new_local_variables,
                        new_form_bindings,
                    ));
                }
            }
        }

        // 更新を適用
        for (key, new_controllers, new_local_variables, new_form_bindings) in updates {
            if let Some(mut binding) = self.ng_include_bindings.get_mut(&key) {
                debug!(
                    "propagate_inheritance: {} -> controllers: {:?}, local_vars: {}, forms: {}",
                    key, new_controllers, new_local_variables.len(), new_form_bindings.len()
                );
                binding.inherited_controllers = new_controllers.clone();
                binding.inherited_local_variables = new_local_variables.clone();
                binding.inherited_form_bindings = new_form_bindings.clone();
            }
            // さらに子への伝播（再帰）
            self.propagate_inheritance_to_children(
                &key,
                &new_controllers,
                &new_local_variables,
                &new_form_bindings,
            );
        }
    }

    /// 親URIを起点として相対パスを解決し、ファイル名を取得
    pub fn resolve_relative_path(parent_uri: &Url, template_path: &str) -> String {
        // クエリパラメータを除去
        let template_path = template_path.split('?').next().unwrap_or(template_path);

        // 親URIのディレクトリ部分を取得
        let parent_path = parent_uri.path();
        let parent_dir = if let Some(last_slash) = parent_path.rfind('/') {
            &parent_path[..last_slash]
        } else {
            ""
        };

        // 相対パスを解決
        let resolved = if template_path.starts_with('/') {
            // 絶対パスの場合はそのまま
            template_path.to_string()
        } else {
            // 相対パスの場合は親ディレクトリを基準に解決
            let mut parts: Vec<&str> = parent_dir.split('/').filter(|s| !s.is_empty()).collect();

            for segment in template_path.split('/') {
                match segment {
                    ".." => { parts.pop(); }
                    "." | "" => {}
                    _ => parts.push(segment),
                }
            }

            format!("/{}", parts.join("/"))
        };

        // ファイル名部分を抽出
        resolved.rsplit('/').next().unwrap_or(&resolved).to_string()
    }

    /// ng-includeで継承されるコントローラーリストを取得
    pub fn get_inherited_controllers_for_template(&self, uri: &Url) -> Vec<String> {
        let path = uri.path();
        let filename = match path.rsplit('/').next() {
            Some(f) => f,
            None => return Vec::new(),
        };

        // 方法1: 正規化パスでマッチング（パス末尾で比較）
        // 例: URI "/Users/.../static/wf/views/foo.html" と
        //     正規化パス "static/wf/views/foo.html" がマッチ
        for entry in self.ng_include_bindings.iter() {
            let normalized_path = entry.key();
            // URIのパスが正規化パスで終わるかチェック
            if path.ends_with(&format!("/{}", normalized_path)) || path == format!("/{}", normalized_path) {
                return entry.value().inherited_controllers.clone();
            }
        }

        // 方法2: ファイル名のみでマッチング（フォールバック）
        if let Some(binding) = self.ng_include_bindings.get(filename) {
            return binding.inherited_controllers.clone();
        }

        // 方法3: resolved_filenameでマッチング
        for entry in self.ng_include_bindings.iter() {
            if entry.value().resolved_filename == filename {
                return entry.value().inherited_controllers.clone();
            }
        }

        Vec::new()
    }

    /// ng-includeで継承されるローカル変数リストを取得
    pub fn get_inherited_local_variables_for_template(&self, uri: &Url) -> Vec<InheritedLocalVariable> {
        let path = uri.path();
        let filename = match path.rsplit('/').next() {
            Some(f) => f,
            None => return Vec::new(),
        };

        // 方法1: 正規化パスでマッチング（パス末尾で比較）
        for entry in self.ng_include_bindings.iter() {
            let normalized_path = entry.key();
            if path.ends_with(&format!("/{}", normalized_path)) || path == format!("/{}", normalized_path) {
                return entry.value().inherited_local_variables.clone();
            }
        }

        // 方法2: ファイル名のみでマッチング（フォールバック）
        if let Some(binding) = self.ng_include_bindings.get(filename) {
            return binding.inherited_local_variables.clone();
        }

        // 方法3: resolved_filenameでマッチング
        for entry in self.ng_include_bindings.iter() {
            if entry.value().resolved_filename == filename {
                return entry.value().inherited_local_variables.clone();
            }
        }

        Vec::new()
    }

    /// 特定のローカル変数を継承しているテンプレートの参照を取得
    /// 親テンプレートで定義された変数に対して、子テンプレート内の参照も収集する
    pub fn get_inherited_local_variable_references(
        &self,
        parent_uri: &Url,
        var_name: &str,
    ) -> Vec<HtmlLocalVariableReference> {
        let mut result = Vec::new();

        // 指定された変数名のすべての参照を取得
        if let Some(refs) = self.html_local_variable_references.get(var_name) {
            for var_ref in refs.iter() {
                // 親URIと同じファイルはスキップ（すでに収集済み）
                if &var_ref.uri == parent_uri {
                    continue;
                }

                // このファイルが指定された変数を継承しているか確認
                let inherited = self.get_inherited_local_variables_for_template(&var_ref.uri);
                let inherits_var = inherited.iter().any(|v| {
                    v.name == var_name && &v.uri == parent_uri
                });

                if inherits_var {
                    result.push(var_ref.clone());
                }
            }
        }

        result
    }

    /// 親URIに関連するng-includeバインディングをクリア
    fn clear_ng_include_bindings_for_parent(&self, parent_uri: &Url) {
        let keys_to_remove: Vec<String> = self
            .ng_include_bindings
            .iter()
            .filter(|entry| &entry.value().parent_uri == parent_uri)
            .map(|entry| entry.key().clone())
            .collect();

        for key in keys_to_remove {
            self.ng_include_bindings.remove(&key);
        }
    }

    /// 子ファイルをng-includeしている親ファイルのリストを取得
    /// 返り値: (親ファイルのURI, ng-includeがある行番号)のリスト
    pub fn get_parent_templates_for_child(&self, uri: &Url) -> Vec<(Url, u32)> {
        let path = uri.path();
        let filename = match path.rsplit('/').next() {
            Some(f) => f,
            None => return Vec::new(),
        };

        let mut result = Vec::new();

        for entry in self.ng_include_bindings.iter() {
            let binding = entry.value();
            let normalized_path = entry.key();

            // マッチング方法1: URIのパスが正規化パスで終わる
            let matches_normalized = path.ends_with(&format!("/{}", normalized_path))
                || path == format!("/{}", normalized_path);

            // マッチング方法2: ファイル名が一致
            let matches_filename = normalized_path == filename;

            // マッチング方法3: resolved_filenameが一致
            let matches_resolved = binding.resolved_filename == filename;

            if matches_normalized || matches_filename || matches_resolved {
                result.push((binding.parent_uri.clone(), binding.line));
            }
        }

        result
    }

    /// 親ファイル内のng-includeの一覧を取得
    /// 返り値: (ng-includeがある行番号, テンプレートパス, 解決されたURI)のリスト
    pub fn get_ng_includes_in_file(&self, uri: &Url) -> Vec<(u32, String, Option<Url>)> {
        let mut result = Vec::new();

        for entry in self.ng_include_bindings.iter() {
            let binding = entry.value();
            if &binding.parent_uri == uri {
                let resolved_uri = self.resolve_template_uri(&binding.template_path);
                result.push((binding.line, binding.template_path.clone(), resolved_uri));
            }
        }

        // 行番号でソート
        result.sort_by_key(|(line, _, _)| *line);

        result
    }

    // ========== HTML解析関連 ==========

    /// HTML内のng-controllerスコープを追加
    pub fn add_html_controller_scope(&self, scope: HtmlControllerScope) {
        let uri = scope.uri.clone();
        self.html_controller_scopes.entry(uri).or_default().push(scope);
    }

    /// 指定URIのHTML内の全ng-controllerスコープを取得
    pub fn get_all_html_controller_scopes(&self, uri: &Url) -> Vec<HtmlControllerScope> {
        self.html_controller_scopes
            .get(uri)
            .map(|scopes| scopes.value().clone())
            .unwrap_or_default()
    }

    /// HTML内のスコープ参照を追加
    pub fn add_html_scope_reference(&self, reference: HtmlScopeReference) {
        let uri = reference.uri.clone();
        self.html_scope_references.entry(uri).or_default().push(reference);
    }

    /// テスト用: 指定URIの全HTMLスコープ参照を取得
    #[cfg(test)]
    pub fn html_scope_references_for_test(&self, uri: &Url) -> Option<Vec<HtmlScopeReference>> {
        self.html_scope_references.get(uri).map(|v| v.value().clone())
    }

    /// 指定位置のHTML内コントローラー名を取得（ネストされた場合は最も内側のスコープ）
    pub fn get_html_controller_at(&self, uri: &Url, line: u32) -> Option<String> {
        if let Some(scopes) = self.html_controller_scopes.get(uri) {
            let mut best_match: Option<&HtmlControllerScope> = None;
            for scope in scopes.iter() {
                if line >= scope.start_line && line <= scope.end_line {
                    if let Some(current_best) = best_match {
                        // より狭いスコープを優先（ネストされたng-controller）
                        if scope.start_line >= current_best.start_line
                            && scope.end_line <= current_best.end_line
                        {
                            best_match = Some(scope);
                        }
                    } else {
                        best_match = Some(scope);
                    }
                }
            }
            return best_match.map(|s| s.controller_name.clone());
        }
        None
    }

    /// 指定位置のHTML内の全コントローラーを取得（外側から内側への順）
    pub fn get_html_controllers_at(&self, uri: &Url, line: u32) -> Vec<String> {
        let mut matching_scopes: Vec<HtmlControllerScope> = Vec::new();

        if let Some(scopes) = self.html_controller_scopes.get(uri) {
            for scope in scopes.iter() {
                if line >= scope.start_line && line <= scope.end_line {
                    matching_scopes.push(scope.clone());
                }
            }
        }

        // スコープの開始行でソート（外側のスコープが先になる）
        matching_scopes.sort_by(|a, b| {
            a.start_line.cmp(&b.start_line)
                .then_with(|| b.end_line.cmp(&a.end_line))
        });

        matching_scopes.iter().map(|s| s.controller_name.clone()).collect()
    }

    /// 指定位置のHTML内でaliasに対応するコントローラー名を取得
    /// 例: <div ng-controller="UserCtrl as vm">内で"vm"を渡すと"UserCtrl"を返す
    pub fn resolve_controller_by_alias(&self, uri: &Url, line: u32, alias: &str) -> Option<String> {
        if let Some(scopes) = self.html_controller_scopes.get(uri) {
            // 最も内側のスコープを優先するため、逆順で探す
            let mut best_match: Option<&HtmlControllerScope> = None;
            for scope in scopes.iter() {
                if line >= scope.start_line && line <= scope.end_line {
                    if let Some(ref scope_alias) = scope.alias {
                        if scope_alias == alias {
                            if let Some(current_best) = best_match {
                                // より狭いスコープを優先（ネストされたng-controller）
                                if scope.start_line >= current_best.start_line
                                    && scope.end_line <= current_best.end_line
                                {
                                    best_match = Some(scope);
                                }
                            } else {
                                best_match = Some(scope);
                            }
                        }
                    }
                }
            }
            return best_match.map(|s| s.controller_name.clone());
        }
        None
    }

    /// 指定位置のHTML内の全aliasマッピングを取得（alias -> controller_name）
    pub fn get_html_alias_mappings(&self, uri: &Url, line: u32) -> std::collections::HashMap<String, String> {
        let mut mappings = std::collections::HashMap::new();

        if let Some(scopes) = self.html_controller_scopes.get(uri) {
            for scope in scopes.iter() {
                if line >= scope.start_line && line <= scope.end_line {
                    if let Some(ref alias) = scope.alias {
                        mappings.insert(alias.clone(), scope.controller_name.clone());
                    }
                }
            }
        }

        mappings
    }

    /// 指定位置のHTMLスコープ参照を取得
    pub fn find_html_scope_reference_at(
        &self,
        uri: &Url,
        line: u32,
        col: u32,
    ) -> Option<HtmlScopeReference> {
        if let Some(refs) = self.html_scope_references.get(uri) {
            for r in refs.iter() {
                if self.is_position_in_range(
                    line, col,
                    r.start_line, r.start_col,
                    r.end_line, r.end_col,
                ) {
                    return Some(r.clone());
                }
            }
        }
        None
    }

    // ========== HTMLローカル変数関連 ==========

    /// HTMLローカル変数定義を追加
    pub fn add_html_local_variable(&self, variable: HtmlLocalVariable) {
        let uri = variable.uri.clone();
        let mut entry = self.html_local_variables.entry(uri).or_default();
        // 重複チェック
        let is_duplicate = entry.iter().any(|v| {
            v.name == variable.name
                && v.name_start_line == variable.name_start_line
                && v.name_start_col == variable.name_start_col
        });
        if !is_duplicate {
            entry.push(variable);
        }
    }

    /// HTMLローカル変数参照を追加
    pub fn add_html_local_variable_reference(&self, reference: HtmlLocalVariableReference) {
        let var_name = reference.variable_name.clone();
        let mut entry = self.html_local_variable_references.entry(var_name).or_default();
        // 重複チェック
        let is_duplicate = entry.iter().any(|r| {
            r.uri == reference.uri
                && r.start_line == reference.start_line
                && r.start_col == reference.start_col
        });
        if !is_duplicate {
            entry.push(reference);
        }
    }

    /// 指定位置で有効なローカル変数を取得
    pub fn get_local_variables_at(&self, uri: &Url, line: u32) -> Vec<HtmlLocalVariable> {
        self.html_local_variables
            .get(uri)
            .map(|vars| {
                vars.iter()
                    .filter(|v| line >= v.scope_start_line && line <= v.scope_end_line)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// ローカル変数の定義を取得
    /// 同名変数がネストしている場合は最も内側のスコープを優先
    /// 継承されたローカル変数（ng-include経由）もチェック
    pub fn find_local_variable_definition(
        &self,
        uri: &Url,
        variable_name: &str,
        line: u32,
    ) -> Option<HtmlLocalVariable> {
        // まず現在のファイル内のローカル変数をチェック
        if let Some(var) = self.html_local_variables.get(uri).and_then(|vars| {
            vars.iter()
                .filter(|v| {
                    v.name == variable_name
                        && line >= v.scope_start_line
                        && line <= v.scope_end_line
                })
                // 最も内側のスコープを優先（同名変数がネストしている場合）
                .max_by_key(|v| v.scope_start_line)
                .cloned()
        }) {
            return Some(var);
        }

        // 継承されたローカル変数をチェック（ng-include経由）
        let inherited = self.get_inherited_local_variables_for_template(uri);
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

    /// ローカル変数の全参照を取得（スコープ内のみ）
    pub fn get_local_variable_references(
        &self,
        uri: &Url,
        variable_name: &str,
        scope_start_line: u32,
        scope_end_line: u32,
    ) -> Vec<HtmlLocalVariableReference> {
        self.html_local_variable_references
            .get(variable_name)
            .map(|refs| {
                refs.iter()
                    .filter(|r| {
                        &r.uri == uri
                            && r.start_line >= scope_start_line
                            && r.start_line <= scope_end_line
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 指定位置のローカル変数参照を検索
    pub fn find_html_local_variable_at(
        &self,
        uri: &Url,
        line: u32,
        col: u32,
    ) -> Option<HtmlLocalVariableReference> {
        for entry in self.html_local_variable_references.iter() {
            for r in entry.value() {
                if &r.uri == uri
                    && self.is_position_in_range(
                        line,
                        col,
                        r.start_line,
                        r.start_col,
                        r.end_line,
                        r.end_col,
                    )
                {
                    return Some(r.clone());
                }
            }
        }
        None
    }

    /// 指定位置のローカル変数定義を検索（定義位置にカーソルがある場合）
    pub fn find_html_local_variable_definition_at(
        &self,
        uri: &Url,
        line: u32,
        col: u32,
    ) -> Option<HtmlLocalVariable> {
        self.html_local_variables.get(uri).and_then(|vars| {
            vars.iter()
                .filter(|v| {
                    self.is_position_in_range(
                        line,
                        col,
                        v.name_start_line,
                        v.name_start_col,
                        v.name_end_line,
                        v.name_end_col,
                    )
                })
                .cloned()
                .next()
        })
    }

    // ========== HTMLフォームバインディング関連 ==========

    /// HTMLフォームバインディングを追加
    pub fn add_html_form_binding(&self, binding: HtmlFormBinding) {
        let uri = binding.uri.clone();
        let mut entry = self.html_form_bindings.entry(uri).or_default();
        // 重複チェック
        let is_duplicate = entry.iter().any(|b| {
            b.name == binding.name
                && b.name_start_line == binding.name_start_line
                && b.name_start_col == binding.name_start_col
        });
        if !is_duplicate {
            entry.push(binding);
        }
    }

    /// 指定位置で有効なフォームバインディングを取得
    pub fn get_form_bindings_at(&self, uri: &Url, line: u32) -> Vec<HtmlFormBinding> {
        self.html_form_bindings
            .get(uri)
            .map(|bindings| {
                bindings
                    .iter()
                    .filter(|b| line >= b.scope_start_line && line <= b.scope_end_line)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// フォームバインディングの定義を取得
    /// 同名フォームがネストしている場合は最も内側のスコープを優先
    /// 継承されたフォームバインディング（ng-include経由）もチェック
    pub fn find_form_binding_definition(
        &self,
        uri: &Url,
        form_name: &str,
        line: u32,
    ) -> Option<HtmlFormBinding> {
        // まず現在のファイル内のフォームバインディングをチェック
        if let Some(binding) = self.html_form_bindings.get(uri).and_then(|bindings| {
            bindings
                .iter()
                .filter(|b| {
                    b.name == form_name
                        && line >= b.scope_start_line
                        && line <= b.scope_end_line
                })
                // 最も内側のスコープを優先（同名フォームがネストしている場合）
                .max_by_key(|b| b.scope_start_line)
                .cloned()
        }) {
            return Some(binding);
        }

        // 継承されたフォームバインディングをチェック（ng-include経由）
        let inherited = self.get_inherited_form_bindings_for_template(uri);
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

    /// 指定位置のフォームバインディング定義を検索（定義位置にカーソルがある場合）
    pub fn find_html_form_binding_at(
        &self,
        uri: &Url,
        line: u32,
        col: u32,
    ) -> Option<HtmlFormBinding> {
        self.html_form_bindings.get(uri).and_then(|bindings| {
            bindings
                .iter()
                .filter(|b| {
                    self.is_position_in_range(
                        line,
                        col,
                        b.name_start_line,
                        b.name_start_col,
                        b.name_end_line,
                        b.name_end_col,
                    )
                })
                .cloned()
                .next()
        })
    }

    /// ng-includeで継承されるフォームバインディングリストを取得
    pub fn get_inherited_form_bindings_for_template(&self, uri: &Url) -> Vec<InheritedFormBinding> {
        let path = uri.path();
        let filename = match path.rsplit('/').next() {
            Some(f) => f,
            None => return Vec::new(),
        };

        // 方法1: 正規化パスでマッチング（パス末尾で比較）
        for entry in self.ng_include_bindings.iter() {
            let normalized_path = entry.key();
            if path.ends_with(&format!("/{}", normalized_path)) || path == format!("/{}", normalized_path) {
                return entry.value().inherited_form_bindings.clone();
            }
        }

        // 方法2: ファイル名のみでマッチング（フォールバック）
        if let Some(binding) = self.ng_include_bindings.get(filename) {
            return binding.inherited_form_bindings.clone();
        }

        // 方法3: resolved_filenameでマッチング
        for entry in self.ng_include_bindings.iter() {
            if entry.value().resolved_filename == filename {
                return entry.value().inherited_form_bindings.clone();
            }
        }

        Vec::new()
    }

    /// HTMLファイルに対応するコントローラー名を解決
    /// 1. HTML内のng-controllerスコープ
    /// 2. テンプレートバインディング（$routeProvider, $uibModal）
    pub fn resolve_controller_for_html(&self, uri: &Url, line: u32) -> Option<String> {
        // 1. HTML内のng-controllerを優先
        if let Some(controller) = self.get_html_controller_at(uri, line) {
            return Some(controller);
        }
        // 2. テンプレートバインディングから検索
        self.get_controller_for_template(uri)
    }

    /// HTMLファイルに対応する全コントローラー名を解決（外側から内側への順）
    /// ng-includeで継承されたコントローラーも含む
    pub fn resolve_controllers_for_html(&self, uri: &Url, line: u32) -> Vec<String> {
        let mut controllers = Vec::new();

        // 1. ng-includeで継承されたコントローラー（親HTMLから）
        let inherited = self.get_inherited_controllers_for_template(uri);
        controllers.extend(inherited);

        // 2. このHTML内のng-controllerスコープ
        let local_controllers = self.get_html_controllers_at(uri, line);
        controllers.extend(local_controllers);

        // 3. テンプレートバインディング経由のコントローラー（継承がない場合のみ）
        if controllers.is_empty() {
            if let Some(controller) = self.get_controller_for_template(uri) {
                controllers.push(controller);
            }
        }

        // 重複を除去（順序は保持）
        let mut seen = std::collections::HashSet::new();
        controllers.retain(|c| seen.insert(c.clone()));

        controllers
    }

    /// テンプレートパスからURIを解決
    /// 例: "static/wf/views/foo.html" -> "file:///path/to/project/static/wf/views/foo.html"
    pub fn resolve_template_uri(&self, template_path: &str) -> Option<Url> {
        let normalized_path = Self::normalize_template_path(template_path);

        // html_controller_scopesのキー（URI）から検索
        for entry in self.html_controller_scopes.iter() {
            let uri = entry.key();
            let path = uri.path();
            if path.ends_with(&format!("/{}", normalized_path)) || path.ends_with(&normalized_path) {
                return Some(uri.clone());
            }
        }

        // html_scope_referencesのキー（URI）からも検索
        for entry in self.html_scope_references.iter() {
            let uri = entry.key();
            let path = uri.path();
            if path.ends_with(&format!("/{}", normalized_path)) || path.ends_with(&normalized_path) {
                return Some(uri.clone());
            }
        }

        // ng_include_bindingsから検索
        if let Some(binding) = self.ng_include_bindings.get(&normalized_path) {
            // parent_uriから相対パスを解決してURIを構築
            let parent_uri = &binding.parent_uri;
            let parent_path = parent_uri.path();
            if let Some(last_slash) = parent_path.rfind('/') {
                let parent_dir = &parent_path[..last_slash];
                let resolved_path = format!("{}/{}", parent_dir, normalized_path);
                if let Ok(uri) = Url::parse(&format!("{}://{}{}", parent_uri.scheme(), parent_uri.authority(), resolved_path)) {
                    return Some(uri);
                }
            }
        }

        None
    }

    /// 指定URIのドキュメントシンボル一覧を取得
    pub fn get_document_symbols(&self, uri: &Url) -> Vec<Symbol> {
        use super::symbol::SymbolKind;

        let mut symbols = Vec::new();

        // 該当URIの定義を収集
        for entry in self.definitions.iter() {
            for symbol in entry.value() {
                if &symbol.uri == uri {
                    symbols.push(symbol.clone());
                }
            }
        }

        // HTMLファイルの場合、html_controller_scopesからもシンボルを作成
        if let Some(scopes) = self.html_controller_scopes.get(uri) {
            for scope in scopes.iter() {
                let symbol = Symbol {
                    name: scope.controller_name.clone(),
                    kind: SymbolKind::Controller,
                    uri: scope.uri.clone(),
                    start_line: scope.start_line,
                    start_col: 0,
                    end_line: scope.end_line,
                    end_col: 0,
                    name_start_line: scope.start_line,
                    name_start_col: 0,
                    name_end_line: scope.start_line,
                    name_end_col: scope.controller_name.len() as u32,
                    docs: Some("ng-controller".to_string()),
                    parameters: None,
                };
                symbols.push(symbol);
            }
        }

        // HTMLファイルの場合、html_scope_referencesからもシンボルを作成
        if let Some(refs) = self.html_scope_references.get(uri) {
            for r in refs.iter() {
                let symbol = Symbol {
                    name: r.property_path.clone(),
                    kind: SymbolKind::ScopeProperty,
                    uri: r.uri.clone(),
                    start_line: r.start_line,
                    start_col: r.start_col,
                    end_line: r.end_line,
                    end_col: r.end_col,
                    name_start_line: r.start_line,
                    name_start_col: r.start_col,
                    name_end_line: r.end_line,
                    name_end_col: r.end_col,
                    docs: None,
                    parameters: None,
                };
                symbols.push(symbol);
            }
        }

        // 開始行でソート
        symbols.sort_by(|a, b| {
            a.start_line
                .cmp(&b.start_line)
                .then(a.start_col.cmp(&b.start_col))
        });

        symbols
    }

    pub fn find_symbol_at_position(&self, uri: &Url, line: u32, col: u32) -> Option<String> {
        let mut best_match: Option<(String, u32)> = None; // (name, range_size)

        // Check definitions - use name position for matching (not definition position)
        for entry in self.definitions.iter() {
            for symbol in entry.value() {
                // シンボル名の位置（name_start_*, name_end_*）を使って検索
                if &symbol.uri == uri && self.is_position_in_range(
                    line, col,
                    symbol.name_start_line, symbol.name_start_col,
                    symbol.name_end_line, symbol.name_end_col,
                ) {
                    debug!(
                        "find_symbol_at_position: definition match '{}' at {}:{}-{}:{}",
                        symbol.name,
                        symbol.name_start_line, symbol.name_start_col,
                        symbol.name_end_line, symbol.name_end_col
                    );
                    let range_size = self.calculate_range_size(
                        symbol.name_start_line, symbol.name_start_col,
                        symbol.name_end_line, symbol.name_end_col,
                    );
                    if best_match.is_none() || range_size < best_match.as_ref().unwrap().1 {
                        best_match = Some((symbol.name.clone(), range_size));
                    }
                }
            }
        }

        // Check references - find the smallest matching range
        for entry in self.references.iter() {
            for reference in entry.value() {
                if &reference.uri == uri && self.is_position_in_range(
                    line, col,
                    reference.start_line, reference.start_col,
                    reference.end_line, reference.end_col,
                ) {
                    debug!(
                        "find_symbol_at_position: reference match '{}' at {}:{}-{}:{}",
                        reference.name,
                        reference.start_line, reference.start_col,
                        reference.end_line, reference.end_col
                    );
                    let range_size = self.calculate_range_size(
                        reference.start_line, reference.start_col,
                        reference.end_line, reference.end_col,
                    );
                    if best_match.is_none() || range_size < best_match.as_ref().unwrap().1 {
                        best_match = Some((reference.name.clone(), range_size));
                    }
                }
            }
        }

        debug!(
            "find_symbol_at_position: result for {}:{} = {:?}",
            line, col, best_match.as_ref().map(|(n, _)| n)
        );
        best_match.map(|(name, _)| name)
    }

    /// 範囲のサイズを計算（行数 * 10000 + 列数で近似）
    fn calculate_range_size(
        &self,
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
    ) -> u32 {
        let line_diff = end_line - start_line;
        let col_diff = if line_diff == 0 {
            end_col - start_col
        } else {
            end_col + (10000 - start_col) // 近似値
        };
        line_diff * 10000 + col_diff
    }

    /// 位置が範囲内にあるかどうかをチェック
    fn is_position_in_range(
        &self,
        line: u32,
        col: u32,
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
    ) -> bool {
        if line < start_line || line > end_line {
            return false;
        }
        if line == start_line && col < start_col {
            return false;
        }
        if line == end_line && col > end_col {
            return false;
        }
        true
    }
}

impl Default for SymbolIndex {
    fn default() -> Self {
        Self::new()
    }
}
