use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

use crate::index::SymbolIndex;

use super::loader::{CachedGlobalData, CachedSymbolData};
use super::metadata::{CacheMetadata, FileMetadata};

/// キャッシュライター
pub struct CacheWriter {
    cache_dir: PathBuf,
}

impl CacheWriter {
    pub fn new(workspace_root: &Path) -> Self {
        Self {
            cache_dir: workspace_root.join(".angularjs-lsp/cache/v1"),
        }
    }

    /// キャッシュディレクトリを作成
    fn ensure_cache_dir(&self) -> std::io::Result<()> {
        if !self.cache_dir.exists() {
            fs::create_dir_all(&self.cache_dir)?;
        }
        Ok(())
    }

    /// 空のCachedSymbolDataを作成
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

    /// インデックス全体をキャッシュに保存
    pub fn save_full(
        &self,
        index: &SymbolIndex,
        file_metadata: &HashMap<PathBuf, FileMetadata>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.ensure_cache_dir()?;

        // メタデータを作成
        let mut metadata = CacheMetadata::new();
        for (path, meta) in file_metadata {
            metadata.files.insert(path.to_string_lossy().to_string(), meta.clone());
        }

        // メタデータを保存
        let metadata_path = self.cache_dir.join("metadata.json");
        let metadata_json = serde_json::to_string_pretty(&metadata)?;
        fs::write(&metadata_path, metadata_json)?;

        // シンボルデータを収集してファイル別にグループ化
        let mut file_data: HashMap<String, CachedSymbolData> = HashMap::new();

        // 定義を収集
        for symbol in index.get_all_definitions() {
            let uri_str = symbol.uri.to_string();
            file_data.entry(uri_str.clone()).or_insert_with(|| Self::empty_cached_data(uri_str)).definitions.push(symbol);
        }

        // 参照を収集（定義があるシンボルの参照のみ）
        for symbol in index.get_all_definitions() {
            for reference in index.get_references(&symbol.name) {
                let uri_str = reference.uri.to_string();
                file_data.entry(uri_str.clone()).or_insert_with(|| Self::empty_cached_data(uri_str)).references.push(reference);
            }
        }

        // コントローラースコープを収集
        for scope in index.get_all_controller_scopes() {
            let uri_str = scope.uri.to_string();
            file_data.entry(uri_str.clone()).or_insert_with(|| Self::empty_cached_data(uri_str)).controller_scopes.push(scope);
        }

        // HTML関連データを収集
        for scope in index.get_all_html_controller_scopes_for_cache() {
            let uri_str = scope.uri.to_string();
            file_data.entry(uri_str.clone()).or_insert_with(|| Self::empty_cached_data(uri_str)).html_controller_scopes.push(scope);
        }

        for reference in index.get_all_html_scope_references_for_cache() {
            let uri_str = reference.uri.to_string();
            file_data.entry(uri_str.clone()).or_insert_with(|| Self::empty_cached_data(uri_str)).html_scope_references.push(reference);
        }

        for variable in index.get_all_html_local_variables_for_cache() {
            let uri_str = variable.uri.to_string();
            file_data.entry(uri_str.clone()).or_insert_with(|| Self::empty_cached_data(uri_str)).html_local_variables.push(variable);
        }

        for reference in index.get_all_html_local_variable_references_for_cache() {
            let uri_str = reference.uri.to_string();
            file_data.entry(uri_str.clone()).or_insert_with(|| Self::empty_cached_data(uri_str)).html_local_variable_references.push(reference);
        }

        for binding in index.get_all_html_form_bindings_for_cache() {
            let uri_str = binding.uri.to_string();
            file_data.entry(uri_str.clone()).or_insert_with(|| Self::empty_cached_data(uri_str)).html_form_bindings.push(binding);
        }

        for reference in index.get_all_html_directive_references_for_cache() {
            let uri_str = reference.uri.to_string();
            file_data.entry(uri_str.clone()).or_insert_with(|| Self::empty_cached_data(uri_str)).html_directive_references.push(reference);
        }

        // シンボルデータを保存
        let cached_data: Vec<CachedSymbolData> = file_data.into_values().collect();
        let data = bincode::serialize(&cached_data)?;
        let data_path = self.cache_dir.join("symbols.bin");
        fs::write(&data_path, data)?;

        // グローバルデータを保存
        self.save_global_data(index)?;

        let html_scopes: usize = cached_data.iter().map(|e| e.html_controller_scopes.len()).sum();
        let html_refs: usize = cached_data.iter().map(|e| e.html_scope_references.len()).sum();

        info!(
            "Saved cache: {} files, {} bytes, {} html_scopes, {} html_refs",
            metadata.files.len(),
            fs::metadata(&data_path).map(|m| m.len()).unwrap_or(0),
            html_scopes,
            html_refs
        );

        Ok(())
    }

    /// グローバルデータを保存
    fn save_global_data(&self, index: &SymbolIndex) -> Result<(), Box<dyn std::error::Error>> {
        let global_data = CachedGlobalData {
            template_bindings: index.get_all_template_bindings(),
            ng_include_bindings: index.get_all_ng_include_bindings(),
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
