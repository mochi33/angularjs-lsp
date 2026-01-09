use dashmap::DashMap;
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

    pub fn get_references(&self, name: &str) -> Vec<SymbolReference> {
        self.references
            .get(name)
            .map(|v| v.value().clone())
            .unwrap_or_default()
    }

    /// シンボル名に対応するHTML内の参照を取得
    /// シンボル名の形式: "ControllerName.$scope.propertyPath"
    pub fn get_html_references_for_symbol(&self, symbol_name: &str) -> Vec<SymbolReference> {
        // シンボル名からコントローラー名とプロパティパスを抽出
        let (controller_name, property_path) = match self.parse_scope_symbol_name(symbol_name) {
            Some(parsed) => parsed,
            None => return Vec::new(),
        };

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

                // このHTML参照がどのコントローラーに属するか確認
                let controllers = self.resolve_controllers_for_html(uri, html_ref.start_line);
                if controllers.contains(&controller_name.to_string()) {
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

    /// スコープシンボル名をパース: "ControllerName.$scope.propertyPath" -> (ControllerName, propertyPath)
    fn parse_scope_symbol_name(&self, symbol_name: &str) -> Option<(String, String)> {
        let scope_marker = ".$scope.";
        let idx = symbol_name.find(scope_marker)?;
        let controller_name = &symbol_name[..idx];
        let property_path = &symbol_name[idx + scope_marker.len()..];
        Some((controller_name.to_string(), property_path.to_string()))
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
    }

    pub fn remove_document(&self, uri: &Url) {
        self.clear_document(uri);
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
        self.propagate_inheritance_to_children(&normalized_path, &[controller_name]);
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
        let inherited_controllers = binding.inherited_controllers.clone();

        debug!(
            "add_ng_include_binding: {} (resolved: {}) -> {:?}",
            normalized_path, binding.resolved_filename, inherited_controllers
        );
        self.ng_include_bindings.insert(normalized_path.clone(), binding);

        // 子ファイル（normalized_path）が親として登録しているng-include bindingがあれば、
        // その継承情報を更新する（継承チェーンの伝播）
        self.propagate_inheritance_to_children(&normalized_path, &inherited_controllers);
    }

    /// 継承情報を子ファイルのng-include bindingに伝播させる
    fn propagate_inheritance_to_children(&self, child_path: &str, parent_controllers: &[String]) {
        // child_pathをparent_uriとして持つng-include bindingを探す
        let mut updates: Vec<(String, Vec<String>)> = Vec::new();

        for entry in self.ng_include_bindings.iter() {
            let binding = entry.value();
            // parent_uriのパスがchild_pathで終わるかチェック
            let parent_uri_path = binding.parent_uri.path();
            if parent_uri_path.ends_with(&format!("/{}", child_path)) ||
               parent_uri_path.ends_with(child_path) {
                // 継承情報を更新（親のコントローラー + 現在のコントローラー）
                let mut new_inherited = parent_controllers.to_vec();
                // 現在のバインディングが持っているコントローラーを追加（重複除去）
                for ctrl in &binding.inherited_controllers {
                    if !new_inherited.contains(ctrl) {
                        new_inherited.push(ctrl.clone());
                    }
                }
                // 既に同じ場合は更新不要
                if new_inherited != binding.inherited_controllers {
                    updates.push((entry.key().clone(), new_inherited));
                }
            }
        }

        // 更新を適用
        for (key, new_inherited) in updates {
            if let Some(mut binding) = self.ng_include_bindings.get_mut(&key) {
                debug!(
                    "propagate_inheritance: {} -> {:?}",
                    key, new_inherited
                );
                binding.inherited_controllers = new_inherited.clone();
            }
            // さらに子への伝播（再帰）
            self.propagate_inheritance_to_children(&key, &new_inherited);
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
