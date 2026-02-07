use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use tracing::{debug, info};

use crate::index::Index;

use super::metadata::{CacheMetadata, FileMetadata};
use super::schema::{CachedGlobalData, CachedSymbolData};

/// Cache writer
pub struct CacheWriter {
    cache_dir: PathBuf,
}

impl CacheWriter {
    pub fn new(workspace_root: &Path) -> Self {
        Self {
            cache_dir: workspace_root.join(".angularjs-lsp/cache/v1"),
        }
    }

    fn ensure_cache_dir(&self) -> std::io::Result<()> {
        if !self.cache_dir.exists() {
            fs::create_dir_all(&self.cache_dir)?;
        }
        Ok(())
    }

    fn empty_cached_data(uri: String) -> CachedSymbolData {
        CachedSymbolData {
            uri,
            definitions: Vec::new(),
            references: Vec::new(),
            controller_scopes: Vec::new(),
            html_controller_scopes: Vec::new(),
            html_scope_references: Vec::new(),
            html_local_variables: Vec::new(),
            html_local_variable_references: Vec::new(),
            html_form_bindings: Vec::new(),
            html_directive_references: Vec::new(),
        }
    }

    /// Save the entire index to cache
    pub fn save_full(
        &self,
        index: &Index,
        file_metadata: &HashMap<PathBuf, FileMetadata>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.ensure_cache_dir()?;

        // Save metadata
        let mut metadata = CacheMetadata::new();
        for (path, meta) in file_metadata {
            metadata
                .files
                .insert(path.to_string_lossy().to_string(), meta.clone());
        }

        let metadata_path = self.cache_dir.join("metadata.json");
        let metadata_json = serde_json::to_string_pretty(&metadata)?;
        fs::write(&metadata_path, metadata_json)?;

        // Collect symbol data grouped by file
        let mut file_data: HashMap<String, CachedSymbolData> = HashMap::new();

        for symbol in index.definitions.get_all_definitions() {
            let uri_str = symbol.uri.to_string();
            file_data
                .entry(uri_str.clone())
                .or_insert_with(|| Self::empty_cached_data(uri_str))
                .definitions
                .push(symbol);
        }

        for symbol in index.definitions.get_all_definitions() {
            for reference in index.definitions.get_references(&symbol.name) {
                let uri_str = reference.uri.to_string();
                file_data
                    .entry(uri_str.clone())
                    .or_insert_with(|| Self::empty_cached_data(uri_str))
                    .references
                    .push(reference);
            }
        }

        for scope in index.controllers.get_all_controller_scopes() {
            let uri_str = scope.uri.to_string();
            file_data
                .entry(uri_str.clone())
                .or_insert_with(|| Self::empty_cached_data(uri_str))
                .controller_scopes
                .push(scope);
        }

        for scope in index.controllers.get_all_html_controller_scopes_for_cache() {
            let uri_str = scope.uri.to_string();
            file_data
                .entry(uri_str.clone())
                .or_insert_with(|| Self::empty_cached_data(uri_str))
                .html_controller_scopes
                .push(scope);
        }

        for reference in index.html.get_all_html_scope_references_for_cache() {
            let uri_str = reference.uri.to_string();
            file_data
                .entry(uri_str.clone())
                .or_insert_with(|| Self::empty_cached_data(uri_str))
                .html_scope_references
                .push(reference);
        }

        for variable in index.html.get_all_html_local_variables_for_cache() {
            let uri_str = variable.uri.to_string();
            file_data
                .entry(uri_str.clone())
                .or_insert_with(|| Self::empty_cached_data(uri_str))
                .html_local_variables
                .push(variable);
        }

        for reference in index.html.get_all_html_local_variable_references_for_cache() {
            let uri_str = reference.uri.to_string();
            file_data
                .entry(uri_str.clone())
                .or_insert_with(|| Self::empty_cached_data(uri_str))
                .html_local_variable_references
                .push(reference);
        }

        for binding in index.html.get_all_html_form_bindings_for_cache() {
            let uri_str = binding.uri.to_string();
            file_data
                .entry(uri_str.clone())
                .or_insert_with(|| Self::empty_cached_data(uri_str))
                .html_form_bindings
                .push(binding);
        }

        for reference in index.html.get_all_html_directive_references_for_cache() {
            let uri_str = reference.uri.to_string();
            file_data
                .entry(uri_str.clone())
                .or_insert_with(|| Self::empty_cached_data(uri_str))
                .html_directive_references
                .push(reference);
        }

        let cached_data: Vec<CachedSymbolData> = file_data.into_values().collect();
        let data = bincode::serialize(&cached_data)?;
        let data_path = self.cache_dir.join("symbols.bin");
        fs::write(&data_path, &data)?;

        // Save global data
        self.save_global_data(index)?;

        let html_scopes: usize = cached_data
            .iter()
            .map(|e| e.html_controller_scopes.len())
            .sum();
        let html_refs: usize = cached_data
            .iter()
            .map(|e| e.html_scope_references.len())
            .sum();

        info!(
            "Saved cache: {} files, {} bytes, {} html_scopes, {} html_refs",
            metadata.files.len(),
            data.len(),
            html_scopes,
            html_refs
        );

        Ok(())
    }

    fn save_global_data(&self, index: &Index) -> Result<(), Box<dyn std::error::Error>> {
        let global_data = CachedGlobalData {
            template_bindings: index.templates.get_all_template_bindings(),
            ng_include_bindings: index.templates.get_all_ng_include_bindings(),
        };

        let data = bincode::serialize(&global_data)?;
        let global_path = self.cache_dir.join("global.bin");
        fs::write(&global_path, data)?;

        debug!(
            "Saved global cache: {} template_bindings, {} ng_include_bindings",
            global_data.template_bindings.len(),
            global_data.ng_include_bindings.len()
        );

        Ok(())
    }
}
