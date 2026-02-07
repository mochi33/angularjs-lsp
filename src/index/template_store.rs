use dashmap::{DashMap, DashSet};
use tower_lsp::lsp_types::Url;

use crate::model::{
    BindingSource, InheritedFormBinding, InheritedLocalVariable, NgIncludeBinding, NgViewBinding,
    TemplateBinding,
};
use crate::util::normalize_template_path;

/// テンプレートバインディング・ng-include/ng-view の管理ストア
pub struct TemplateStore {
    /// テンプレートバインディング（binding_uri#binding_line#normalized_path -> binding）
    template_bindings: DashMap<String, TemplateBinding>,
    /// ng-includeバインディング（parent_uri#template_path -> binding）
    ng_include_bindings: DashMap<String, NgIncludeBinding>,
    /// ng-include逆引きインデックス: resolved_filename -> Vec<複合キー>
    ng_include_by_filename: DashMap<String, Vec<String>>,
    /// ng-include逆引きインデックス: normalized_template_path -> Vec<複合キー>
    ng_include_by_path: DashMap<String, Vec<String>>,
    /// ng-viewバインディング（parent_uri.to_string() -> binding）
    ng_view_bindings: DashMap<String, NgViewBinding>,
    /// $routeProviderで設定されたテンプレートパスの逆引きインデックス
    route_provider_templates: DashSet<String>,
    /// 再解析が必要なHTMLファイル
    pending_reanalysis: DashSet<Url>,
    /// 解析済みのHTMLファイルのURI
    analyzed_html_files: DashSet<Url>,
}

impl TemplateStore {
    pub fn new() -> Self {
        Self {
            template_bindings: DashMap::new(),
            ng_include_bindings: DashMap::new(),
            ng_include_by_filename: DashMap::new(),
            ng_include_by_path: DashMap::new(),
            ng_view_bindings: DashMap::new(),
            route_provider_templates: DashSet::new(),
            pending_reanalysis: DashSet::new(),
            analyzed_html_files: DashSet::new(),
        }
    }

    // ========== テンプレートバインディング ==========

    pub fn add_template_binding(&self, binding: TemplateBinding) {
        let normalized_path = normalize_template_path(&binding.template_path);
        let controller_name = binding.controller_name.clone();
        let source = binding.source.clone();

        let normalized_binding = TemplateBinding {
            template_path: normalized_path.clone(),
            controller_name: binding.controller_name,
            source: binding.source,
            binding_uri: binding.binding_uri.clone(),
            binding_line: binding.binding_line,
        };
        let binding_key = format!(
            "{}#{}#{}",
            binding.binding_uri.as_str(),
            binding.binding_line,
            normalized_path
        );
        self.template_bindings.insert(binding_key, normalized_binding);

        if source == BindingSource::RouteProvider {
            let filename = normalized_path
                .rsplit('/')
                .next()
                .unwrap_or(&normalized_path);
            self.route_provider_templates
                .insert(filename.to_string());
            self.route_provider_templates
                .insert(normalized_path.clone());
        }

        self.propagate_inheritance_to_children(&normalized_path, &[controller_name], &[], &[]);
    }

    /// URIからコントローラー名を取得（テンプレートバインディング経由）
    pub fn get_controller_for_template(&self, uri: &Url) -> Option<String> {
        let path = uri.path();
        let filename = path.rsplit('/').next()?;

        // パス末尾でマッチング
        for entry in self.template_bindings.iter() {
            let binding = entry.value();
            if path.ends_with(&format!("/{}", binding.template_path))
                || path == format!("/{}", binding.template_path)
            {
                return Some(binding.controller_name.clone());
            }
        }

        // ファイル名のみでマッチング
        for entry in self.template_bindings.iter() {
            let binding = entry.value();
            let binding_filename = binding
                .template_path
                .rsplit('/')
                .next()
                .unwrap_or(&binding.template_path);
            if binding_filename == filename {
                return Some(binding.controller_name.clone());
            }
        }
        None
    }

    /// URIからテンプレートバインディングのソース情報を取得
    pub fn get_template_binding_source(
        &self,
        uri: &Url,
    ) -> Option<(String, BindingSource, Url, u32)> {
        let path = uri.path();
        let filename = path.rsplit('/').next()?;

        let binding = {
            let mut found = None;
            for entry in self.template_bindings.iter() {
                let binding = entry.value();
                if path.ends_with(&format!("/{}", binding.template_path))
                    || path == format!("/{}", binding.template_path)
                {
                    found = Some(binding.clone());
                    break;
                }
            }
            if found.is_none() {
                for entry in self.template_bindings.iter() {
                    let binding = entry.value();
                    let binding_filename = binding
                        .template_path
                        .rsplit('/')
                        .next()
                        .unwrap_or(&binding.template_path);
                    if binding_filename == filename {
                        found = Some(binding.clone());
                        break;
                    }
                }
            }
            found
        }?;

        Some((
            binding.controller_name,
            binding.source,
            binding.binding_uri,
            binding.binding_line,
        ))
    }

    /// URIからテンプレートバインディングの全ソース情報を取得
    pub fn get_all_template_binding_sources(
        &self,
        uri: &Url,
    ) -> Vec<(String, BindingSource, Url, u32)> {
        let path = uri.path();
        let filename = path.rsplit('/').next().unwrap_or("");
        let mut results = Vec::new();

        for entry in self.template_bindings.iter() {
            let binding = entry.value();
            if path.ends_with(&format!("/{}", binding.template_path))
                || path == format!("/{}", binding.template_path)
            {
                results.push((
                    binding.controller_name.clone(),
                    binding.source.clone(),
                    binding.binding_uri.clone(),
                    binding.binding_line,
                ));
            }
        }

        if results.is_empty() {
            for entry in self.template_bindings.iter() {
                let binding = entry.value();
                let binding_filename = binding
                    .template_path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&binding.template_path);
                if binding_filename == filename {
                    results.push((
                        binding.controller_name.clone(),
                        binding.source.clone(),
                        binding.binding_uri.clone(),
                        binding.binding_line,
                    ));
                }
            }
        }

        results
    }

    /// JSファイルURIから、そのファイル内で定義されているテンプレートバインディングを取得
    pub fn get_template_bindings_for_js_file(&self, uri: &Url) -> Vec<TemplateBinding> {
        self.template_bindings
            .iter()
            .filter(|entry| &entry.value().binding_uri == uri)
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// コントローラー名からバインドされているHTMLテンプレートのパスを取得
    pub fn get_templates_for_controller(&self, controller_name: &str) -> Vec<String> {
        let mut templates = Vec::new();
        for entry in self.template_bindings.iter() {
            if entry.value().controller_name == controller_name {
                let path = entry.value().template_path.clone();
                if !templates.contains(&path) {
                    templates.push(path);
                }
            }
        }
        templates
    }

    /// 全テンプレートバインディングを取得（キャッシュ用）
    pub fn get_all_template_bindings(&self) -> Vec<TemplateBinding> {
        self.template_bindings
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    // ========== ng-includeバインディング ==========

    fn make_ng_include_key(parent_uri: &Url, template_path: &str) -> String {
        format!("{}#{}", parent_uri.as_str(), template_path)
    }

    fn extract_template_path_from_key(key: &str) -> &str {
        if let Some(idx) = key.rfind('#') {
            &key[idx + 1..]
        } else {
            key
        }
    }

    pub fn add_ng_include_binding(&self, binding: NgIncludeBinding) {
        let normalized_path = normalize_template_path(&binding.template_path);
        let resolved_filename = binding.resolved_filename.clone();
        let inherited_controllers = binding.inherited_controllers.clone();
        let inherited_local_variables = binding.inherited_local_variables.clone();
        let inherited_form_bindings = binding.inherited_form_bindings.clone();

        let key = Self::make_ng_include_key(&binding.parent_uri, &normalized_path);

        self.ng_include_by_filename
            .entry(resolved_filename.clone())
            .or_default()
            .push(key.clone());
        self.ng_include_by_path
            .entry(normalized_path.clone())
            .or_default()
            .push(key.clone());

        self.ng_include_bindings.insert(key, binding);

        self.queue_child_for_reanalysis(&resolved_filename, &normalized_path);

        self.propagate_inheritance_to_children(
            &normalized_path,
            &inherited_controllers,
            &inherited_local_variables,
            &inherited_form_bindings,
        );
    }

    pub fn add_ng_view_binding(&self, binding: NgViewBinding) {
        let key = binding.parent_uri.to_string();
        self.ng_view_bindings.insert(key, binding);
    }

    fn queue_child_for_reanalysis(&self, resolved_filename: &str, normalized_path: &str) {
        for uri in self.analyzed_html_files.iter() {
            let uri_path = uri.path();
            if uri_path.ends_with(&format!("/{}", resolved_filename))
                || uri_path.ends_with(&format!("/{}", normalized_path))
                || uri_path == format!("/{}", resolved_filename)
                || uri_path == format!("/{}", normalized_path)
            {
                self.pending_reanalysis.insert(uri.clone());
            }
        }
    }

    fn propagate_inheritance_to_children(
        &self,
        child_path: &str,
        parent_controllers: &[String],
        parent_local_variables: &[InheritedLocalVariable],
        parent_form_bindings: &[InheritedFormBinding],
    ) {
        let mut updates: Vec<(
            String,
            Vec<String>,
            Vec<InheritedLocalVariable>,
            Vec<InheritedFormBinding>,
        )> = Vec::new();

        for entry in self.ng_include_bindings.iter() {
            let binding = entry.value();
            let parent_uri_path = binding.parent_uri.path();
            if parent_uri_path.ends_with(&format!("/{}", child_path))
                || parent_uri_path.ends_with(child_path)
            {
                let mut new_controllers = parent_controllers.to_vec();
                for ctrl in &binding.inherited_controllers {
                    if !new_controllers.contains(ctrl) {
                        new_controllers.push(ctrl.clone());
                    }
                }

                let mut new_local_variables = parent_local_variables.to_vec();
                for var in &binding.inherited_local_variables {
                    if !new_local_variables.iter().any(|v| v.name == var.name) {
                        new_local_variables.push(var.clone());
                    }
                }

                let mut new_form_bindings = parent_form_bindings.to_vec();
                for form in &binding.inherited_form_bindings {
                    if !new_form_bindings.iter().any(|f| f.name == form.name) {
                        new_form_bindings.push(form.clone());
                    }
                }

                let controllers_changed = new_controllers != binding.inherited_controllers;
                let local_vars_changed = new_local_variables.len()
                    != binding.inherited_local_variables.len()
                    || !new_local_variables
                        .iter()
                        .all(|v| binding.inherited_local_variables.iter().any(|bv| bv.name == v.name));
                let forms_changed = new_form_bindings.len()
                    != binding.inherited_form_bindings.len()
                    || !new_form_bindings
                        .iter()
                        .all(|f| binding.inherited_form_bindings.iter().any(|bf| bf.name == f.name));

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

        for (key, new_controllers, new_local_variables, new_form_bindings) in updates {
            if let Some(mut binding) = self.ng_include_bindings.get_mut(&key) {
                binding.inherited_controllers = new_controllers.clone();
                binding.inherited_local_variables = new_local_variables.clone();
                binding.inherited_form_bindings = new_form_bindings.clone();
            }
            let child_template_path = Self::extract_template_path_from_key(&key);
            self.propagate_inheritance_to_children(
                child_template_path,
                &new_controllers,
                &new_local_variables,
                &new_form_bindings,
            );
        }
    }

    /// ng-includeで継承されるコントローラーリストを取得
    pub fn get_inherited_controllers_for_template(&self, uri: &Url) -> Vec<String> {
        let mut controllers = Vec::new();
        let keys = self.find_all_ng_include_keys_for_template(uri);
        for key in &keys {
            if let Some(binding) = self.ng_include_bindings.get(key) {
                for controller in &binding.inherited_controllers {
                    if !controllers.contains(controller) {
                        controllers.push(controller.clone());
                    }
                }
            }
        }
        controllers
    }

    /// ng-includeで継承されるローカル変数リストを取得
    pub fn get_inherited_local_variables_for_template(
        &self,
        uri: &Url,
    ) -> Vec<InheritedLocalVariable> {
        let mut variables = Vec::new();
        let keys = self.find_all_ng_include_keys_for_template(uri);
        for key in keys {
            if let Some(binding) = self.ng_include_bindings.get(&key) {
                for var in &binding.inherited_local_variables {
                    if !variables.iter().any(|v: &InheritedLocalVariable| v.name == var.name) {
                        variables.push(var.clone());
                    }
                }
            }
        }
        variables
    }

    /// ng-includeで継承されるフォームバインディングリストを取得
    pub fn get_inherited_form_bindings_for_template(
        &self,
        uri: &Url,
    ) -> Vec<InheritedFormBinding> {
        let mut bindings = Vec::new();
        let keys = self.find_all_ng_include_keys_for_template(uri);
        for key in keys {
            if let Some(binding) = self.ng_include_bindings.get(&key) {
                for form in &binding.inherited_form_bindings {
                    if !bindings.iter().any(|b: &InheritedFormBinding| b.name == form.name) {
                        bindings.push(form.clone());
                    }
                }
            }
        }
        bindings
    }

    fn find_all_ng_include_keys_for_template(&self, uri: &Url) -> Vec<String> {
        let path = uri.path();
        let filename = match path.rsplit('/').next() {
            Some(f) => f,
            None => return Vec::new(),
        };

        let mut keys = Vec::new();

        for entry in self.ng_include_by_path.iter() {
            let template_path = entry.key();
            if path.ends_with(&format!("/{}", template_path))
                || path == format!("/{}", template_path)
            {
                keys.extend(entry.value().iter().cloned());
            }
        }

        if keys.is_empty() {
            if let Some(filename_keys) = self.ng_include_by_filename.get(filename) {
                keys.extend(filename_keys.iter().cloned());
            }
        }

        if keys.is_empty() {
            for entry in self.ng_include_bindings.iter() {
                let binding = entry.value();
                let template_path = Self::extract_template_path_from_key(entry.key());
                let matches_path = path.ends_with(&format!("/{}", template_path))
                    || path == format!("/{}", template_path);
                let matches_resolved = binding.resolved_filename == filename;
                if matches_path || matches_resolved {
                    keys.push(entry.key().clone());
                }
            }
        }

        keys
    }

    /// 子ファイルをng-includeしている親ファイルのリストを取得
    pub fn get_parent_templates_for_child(&self, uri: &Url) -> Vec<(Url, u32)> {
        let path = uri.path();
        let filename = match path.rsplit('/').next() {
            Some(f) => f,
            None => return Vec::new(),
        };

        let mut result = Vec::new();
        for entry in self.ng_include_bindings.iter() {
            let binding = entry.value();
            let template_path = Self::extract_template_path_from_key(entry.key());
            let matches_path = path.ends_with(&format!("/{}", template_path))
                || path == format!("/{}", template_path);
            let matches_resolved = binding.resolved_filename == filename;
            if matches_path || matches_resolved {
                result.push((binding.parent_uri.clone(), binding.line));
            }
        }
        result
    }

    /// 親ファイル内のng-includeの一覧を取得
    pub fn get_ng_includes_in_file(&self, uri: &Url) -> Vec<(u32, String, Option<Url>)> {
        let mut result = Vec::new();
        for entry in self.ng_include_bindings.iter() {
            let binding = entry.value();
            if &binding.parent_uri == uri {
                let resolved_uri = self.resolve_template_uri(&binding.template_path);
                result.push((binding.line, binding.template_path.clone(), resolved_uri));
            }
        }
        result.sort_by_key(|(line, _, _)| *line);
        result
    }

    /// テンプレートパスからURIを解決
    pub fn resolve_template_uri(&self, template_path: &str) -> Option<Url> {
        let normalized_path = normalize_template_path(template_path);

        // ng-include bindingsから検索
        for entry in self.ng_include_bindings.iter() {
            let tp = Self::extract_template_path_from_key(entry.key());
            if tp == normalized_path {
                let binding = entry.value();
                let parent_uri = &binding.parent_uri;
                let parent_path = parent_uri.path();
                if let Some(last_slash) = parent_path.rfind('/') {
                    let parent_dir = &parent_path[..last_slash];
                    let resolved_path = format!("{}/{}", parent_dir, normalized_path);
                    if let Ok(uri) = Url::parse(&format!(
                        "{}://{}{}",
                        parent_uri.scheme(),
                        parent_uri.authority(),
                        resolved_path
                    )) {
                        return Some(uri);
                    }
                }
            }
        }
        None
    }

    /// 全ての$routeProviderテンプレートに対してng-view継承を適用
    pub fn apply_all_ng_view_inheritances(&self) {
        let templates: Vec<String> = self
            .route_provider_templates
            .iter()
            .map(|t| t.clone())
            .collect();

        let mut inherited_controllers = Vec::new();
        let mut inherited_local_variables = Vec::new();
        let mut inherited_form_bindings = Vec::new();

        for entry in self.ng_view_bindings.iter() {
            for controller in &entry.inherited_controllers {
                if !inherited_controllers.contains(controller) {
                    inherited_controllers.push(controller.clone());
                }
            }
            for var in &entry.inherited_local_variables {
                if !inherited_local_variables
                    .iter()
                    .any(|v: &InheritedLocalVariable| v.name == var.name)
                {
                    inherited_local_variables.push(var.clone());
                }
            }
            for form in &entry.inherited_form_bindings {
                if !inherited_form_bindings
                    .iter()
                    .any(|f: &InheritedFormBinding| f.name == form.name)
                {
                    inherited_form_bindings.push(form.clone());
                }
            }
        }

        if inherited_controllers.is_empty() {
            return;
        }

        for template_path in templates {
            let resolved_filename = template_path
                .rsplit('/')
                .next()
                .unwrap_or(&template_path)
                .to_string();

            let binding = NgIncludeBinding {
                parent_uri: Url::parse("file:///ng-view-virtual-parent").unwrap(),
                template_path: template_path.clone(),
                resolved_filename,
                line: 0,
                inherited_controllers: inherited_controllers.clone(),
                inherited_local_variables: inherited_local_variables.clone(),
                inherited_form_bindings: inherited_form_bindings.clone(),
            };
            self.add_ng_include_binding_with_key(format!("ng-view##{}", template_path), binding);
        }
    }

    /// このテンプレートが$routeProviderで設定されたものかどうかを判定
    pub fn is_route_provider_template(&self, uri: &Url) -> bool {
        let path = uri.path();
        let filename = match path.rsplit('/').next() {
            Some(f) => f,
            None => return false,
        };
        self.route_provider_templates.contains(filename)
    }

    /// ng-view継承されるコントローラーリストを取得
    pub fn get_ng_view_inherited_controllers(&self, uri: &Url) -> Vec<String> {
        if !self.is_route_provider_template(uri) {
            return Vec::new();
        }
        let mut controllers = Vec::new();
        for entry in self.ng_view_bindings.iter() {
            for controller in &entry.inherited_controllers {
                if !controllers.contains(controller) {
                    controllers.push(controller.clone());
                }
            }
        }
        controllers
    }

    /// 特定のローカル変数を継承しているテンプレートの参照を取得
    pub fn get_inherited_local_variable_references(
        &self,
        parent_uri: &Url,
        var_name: &str,
        html_local_variable_references: &dashmap::DashMap<String, Vec<crate::model::HtmlLocalVariableReference>>,
    ) -> Vec<crate::model::HtmlLocalVariableReference> {
        let mut result = Vec::new();
        if let Some(refs) = html_local_variable_references.get(var_name) {
            for var_ref in refs.iter() {
                if &var_ref.uri == parent_uri {
                    continue;
                }
                let inherited = self.get_inherited_local_variables_for_template(&var_ref.uri);
                let inherits_var = inherited
                    .iter()
                    .any(|v| v.name == var_name && &v.uri == parent_uri);
                if inherits_var {
                    result.push(var_ref.clone());
                }
            }
        }
        result
    }

    /// 全ng-includeバインディングを取得（キャッシュ用）
    pub fn get_all_ng_include_bindings(&self) -> Vec<(String, NgIncludeBinding)> {
        self.ng_include_bindings
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// キーを指定してng-includeバインディングを追加（キャッシュ復元用）
    pub fn add_ng_include_binding_with_key(&self, key: String, binding: NgIncludeBinding) {
        let resolved_filename = binding.resolved_filename.clone();
        let normalized_path = normalize_template_path(&binding.template_path);

        self.ng_include_by_filename
            .entry(resolved_filename)
            .or_default()
            .push(key.clone());
        self.ng_include_by_path
            .entry(normalized_path)
            .or_default()
            .push(key.clone());

        self.ng_include_bindings.insert(key, binding);
    }

    /// 再解析が必要なURIを取得してキューをクリア
    pub fn take_pending_reanalysis(&self) -> Vec<Url> {
        let uris: Vec<Url> = self.pending_reanalysis.iter().map(|r| r.clone()).collect();
        self.pending_reanalysis.clear();
        uris
    }

    pub fn remove_from_pending_reanalysis(&self, uri: &Url) {
        self.pending_reanalysis.remove(uri);
    }

    pub fn mark_html_analyzed(&self, uri: &Url) {
        self.analyzed_html_files.insert(uri.clone());
    }

    pub fn clear_ng_include_bindings_for_parent(&self, parent_uri: &Url) {
        let entries_to_remove: Vec<(String, String, String)> = self
            .ng_include_bindings
            .iter()
            .filter(|entry| &entry.value().parent_uri == parent_uri)
            .map(|entry| {
                let key = entry.key().clone();
                let resolved_filename = entry.value().resolved_filename.clone();
                let normalized_path =
                    normalize_template_path(&entry.value().template_path);
                (key, resolved_filename, normalized_path)
            })
            .collect();

        for (key, resolved_filename, normalized_path) in entries_to_remove {
            if let Some(mut keys) = self.ng_include_by_filename.get_mut(&resolved_filename) {
                keys.retain(|k| k != &key);
            }
            if let Some(mut keys) = self.ng_include_by_path.get_mut(&normalized_path) {
                keys.retain(|k| k != &key);
            }
            self.ng_include_bindings.remove(&key);
        }
    }

    pub fn clear_document(&self, uri: &Url) {
        self.clear_ng_include_bindings_for_parent(uri);
        self.ng_view_bindings.remove(&uri.to_string());
    }

    pub fn clear_all(&self) {
        self.template_bindings.clear();
        self.ng_include_bindings.clear();
        self.ng_include_by_filename.clear();
        self.ng_include_by_path.clear();
        self.ng_view_bindings.clear();
        self.route_provider_templates.clear();
        self.pending_reanalysis.clear();
        self.analyzed_html_files.clear();
    }
}

impl Default for TemplateStore {
    fn default() -> Self {
        Self::new()
    }
}
