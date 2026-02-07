use dashmap::DashMap;
use tower_lsp::lsp_types::Url;

use crate::model::ComponentTemplateUrl;
use crate::util::normalize_template_path;

/// コンポーネントテンプレートの管理ストア
pub struct ComponentStore {
    /// コンポーネントのtemplateUrl情報（URI -> Vec<ComponentTemplateUrl>）
    component_template_urls: DashMap<Url, Vec<ComponentTemplateUrl>>,
    /// コンポーネントテンプレートバインディング逆引き（normalized_path -> ComponentTemplateUrl）
    component_template_bindings: DashMap<String, ComponentTemplateUrl>,
}

impl ComponentStore {
    pub fn new() -> Self {
        Self {
            component_template_urls: DashMap::new(),
            component_template_bindings: DashMap::new(),
        }
    }

    pub fn add_component_template_url(&self, template_url: ComponentTemplateUrl) {
        let uri = template_url.uri.clone();
        let normalized_path = normalize_template_path(&template_url.template_path);
        self.component_template_bindings
            .insert(normalized_path, template_url.clone());
        self.component_template_urls
            .entry(uri)
            .or_default()
            .push(template_url);
    }

    pub fn get_component_template_urls(&self, uri: &Url) -> Vec<ComponentTemplateUrl> {
        self.component_template_urls
            .get(uri)
            .map(|v| v.value().clone())
            .unwrap_or_default()
    }

    /// HTMLテンプレートURIからコンポーネントバインディングを取得
    pub fn get_component_binding_for_template(
        &self,
        uri: &Url,
    ) -> Option<ComponentTemplateUrl> {
        let path = uri.path();
        for entry in self.component_template_bindings.iter() {
            if path.ends_with(entry.key().as_str()) {
                return Some(entry.value().clone());
            }
        }
        None
    }

    /// コンポーネントテンプレートのcontrollerAsエイリアスを解決
    pub fn resolve_component_controller_by_alias(
        &self,
        uri: &Url,
        alias: &str,
    ) -> Option<String> {
        if let Some(binding) = self.get_component_binding_for_template(uri) {
            if binding.controller_as == alias {
                return binding.controller_name.clone();
            }
        }
        None
    }

    pub fn clear_document(&self, uri: &Url) {
        if let Some(templates) = self.component_template_urls.get(uri) {
            for template in templates.iter() {
                let normalized_path =
                    normalize_template_path(&template.template_path);
                self.component_template_bindings.remove(&normalized_path);
            }
        }
        self.component_template_urls.remove(uri);
    }

    pub fn clear_all(&self) {
        self.component_template_urls.clear();
        self.component_template_bindings.clear();
    }
}

impl Default for ComponentStore {
    fn default() -> Self {
        Self::new()
    }
}
