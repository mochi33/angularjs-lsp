use dashmap::DashMap;
use tower_lsp::lsp_types::Url;

use crate::model::{ControllerScope, HtmlControllerScope};

/// JS/HTMLコントローラースコープの管理ストア
pub struct ControllerStore {
    /// JSファイル内のコントローラースコープ（URI -> Vec<ControllerScope>）
    controller_scopes: DashMap<Url, Vec<ControllerScope>>,
    /// HTML内のng-controllerスコープ（URI -> Vec<HtmlControllerScope>）
    html_controller_scopes: DashMap<Url, Vec<HtmlControllerScope>>,
}

impl ControllerStore {
    pub fn new() -> Self {
        Self {
            controller_scopes: DashMap::new(),
            html_controller_scopes: DashMap::new(),
        }
    }

    // ========== JS Controller Scopes ==========

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

    /// 全コントローラースコープを取得（キャッシュ用）
    pub fn get_all_controller_scopes(&self) -> Vec<ControllerScope> {
        self.controller_scopes
            .iter()
            .flat_map(|entry| entry.value().clone())
            .collect()
    }

    // ========== HTML Controller Scopes ==========

    pub fn add_html_controller_scope(&self, scope: HtmlControllerScope) {
        let uri = scope.uri.clone();
        self.html_controller_scopes
            .entry(uri)
            .or_default()
            .push(scope);
    }

    /// 指定URIのHTML内の全ng-controllerスコープを取得
    pub fn get_all_html_controller_scopes(&self, uri: &Url) -> Vec<HtmlControllerScope> {
        self.html_controller_scopes
            .get(uri)
            .map(|scopes| scopes.value().clone())
            .unwrap_or_default()
    }

    /// 指定位置のHTML内コントローラー名を取得（最も内側のスコープ）
    pub fn get_html_controller_at(&self, uri: &Url, line: u32) -> Option<String> {
        if let Some(scopes) = self.html_controller_scopes.get(uri) {
            let mut best_match: Option<&HtmlControllerScope> = None;
            for scope in scopes.iter() {
                if line >= scope.start_line && line <= scope.end_line {
                    if let Some(current_best) = best_match {
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

        matching_scopes.sort_by(|a, b| {
            a.start_line
                .cmp(&b.start_line)
                .then_with(|| b.end_line.cmp(&a.end_line))
        });

        matching_scopes
            .iter()
            .map(|s| s.controller_name.clone())
            .collect()
    }

    /// 指定位置のHTML内でaliasに対応するコントローラー名を取得
    pub fn resolve_controller_by_alias(
        &self,
        uri: &Url,
        line: u32,
        alias: &str,
    ) -> Option<String> {
        if let Some(scopes) = self.html_controller_scopes.get(uri) {
            let mut best_match: Option<&HtmlControllerScope> = None;
            for scope in scopes.iter() {
                if line >= scope.start_line && line <= scope.end_line {
                    if let Some(ref scope_alias) = scope.alias {
                        if scope_alias == alias {
                            if let Some(current_best) = best_match {
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
            if let Some(matched) = best_match {
                return Some(matched.controller_name.clone());
            }
        }
        None
    }

    /// 指定位置のHTML内の全aliasマッピングを取得
    pub fn get_html_alias_mappings(
        &self,
        uri: &Url,
        line: u32,
    ) -> std::collections::HashMap<String, String> {
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

    /// コントローラー名からバインドされているHTMLテンプレートのパスを取得
    pub fn get_html_templates_for_controller(&self, controller_name: &str) -> Vec<String> {
        let mut templates = Vec::new();
        for entry in self.html_controller_scopes.iter() {
            for scope in entry.value() {
                if scope.controller_name == controller_name {
                    let path = entry.key().path().to_string();
                    if !templates.contains(&path) {
                        templates.push(path);
                    }
                }
            }
        }
        templates
    }

    /// 全HTMLコントローラースコープを取得（キャッシュ用）
    pub fn get_all_html_controller_scopes_for_cache(&self) -> Vec<HtmlControllerScope> {
        self.html_controller_scopes
            .iter()
            .flat_map(|entry| entry.value().clone())
            .collect()
    }

    /// URIのHTML Controller Scopeキーをイテレート（テンプレートURI解決用）
    pub fn html_controller_scope_uris(&self) -> Vec<Url> {
        self.html_controller_scopes
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    pub fn clear_document(&self, uri: &Url) {
        self.controller_scopes.remove(uri);
        self.html_controller_scopes.remove(uri);
    }

    pub fn clear_all(&self) {
        self.controller_scopes.clear();
        self.html_controller_scopes.clear();
    }
}

impl Default for ControllerStore {
    fn default() -> Self {
        Self::new()
    }
}
