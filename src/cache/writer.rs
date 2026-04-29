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
        // InterpolateStore の JS 検出値を URI string 化して保存
        let interpolate_symbols: Vec<(String, Option<String>, Option<String>)> = index
            .interpolate
            .iter_js_detected_for_cache()
            .into_iter()
            .map(|(uri, start, end)| (uri.to_string(), start, end))
            .collect();

        let global_data = CachedGlobalData {
            template_bindings: index.templates.get_all_template_bindings(),
            ng_include_bindings: index.templates.get_all_ng_include_bindings(),
            interpolate_symbols,
        };

        let data = bincode::serialize(&global_data)?;
        let global_path = self.cache_dir.join("global.bin");
        fs::write(&global_path, data)?;

        debug!(
            "Saved global cache: {} template_bindings, {} ng_include_bindings, {} interpolate_symbols",
            global_data.template_bindings.len(),
            global_data.ng_include_bindings.len(),
            global_data.interpolate_symbols.len()
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use tempfile::TempDir;
    use tower_lsp::lsp_types::Url;

    use crate::cache::loader::CacheLoader;

    /// `$interpolateProvider` 検出値を save → load で復元できることを確認。
    /// これがないとカスタム interpolate 記号を使うプロジェクトで cache hit 起動時に
    /// HTML 解析がデフォルト `{{ }}` で動いてしまう。
    #[test]
    fn interpolate_symbols_round_trip() {
        let tmp = TempDir::new().unwrap();
        let workspace_root = tmp.path();

        // 元 Index に interpolate 検出値を入れる
        let original = Index::new();
        let uri_a = Url::parse("file:///a.js").unwrap();
        let uri_b = Url::parse("file:///b.js").unwrap();
        original
            .interpolate
            .set_start_symbol(uri_a.clone(), "<%".to_string());
        original
            .interpolate
            .set_end_symbol(uri_a.clone(), "%>".to_string());
        original
            .interpolate
            .set_start_symbol(uri_b.clone(), "[[".to_string());
        // uri_b は end_symbol を宣言していない (片方だけのケース)

        // 書き出し
        let writer = CacheWriter::new(workspace_root);
        let metadata = HashMap::new();
        writer.save_full(&original, &metadata).unwrap();

        // 別 Index にロード
        let restored = Index::new();
        let loader = CacheLoader::new(workspace_root);
        let valid_files: HashSet<PathBuf> = HashSet::new(); // 全エントリ skip され得るが global は無関係
        loader.load(&restored, &valid_files).unwrap();

        // resolved() が同じ結果になっていること
        assert_eq!(original.interpolate.resolved(), restored.interpolate.resolved());
        assert_eq!(restored.interpolate.resolved().0, "<%".to_string());
        assert_eq!(restored.interpolate.resolved().1, "%>".to_string());

        // 各 URI のエントリも復元されていること
        let mut entries = restored.interpolate.iter_js_detected_for_cache();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, uri_a);
        assert_eq!(entries[0].1, Some("<%".to_string()));
        assert_eq!(entries[0].2, Some("%>".to_string()));
        assert_eq!(entries[1].0, uri_b);
        assert_eq!(entries[1].1, Some("[[".to_string()));
        assert_eq!(entries[1].2, None);
    }

    #[test]
    fn empty_interpolate_round_trip() {
        // 検出値が無いケースでも save/load が壊れないことを確認 (後方互換性)
        let tmp = TempDir::new().unwrap();
        let workspace_root = tmp.path();

        let original = Index::new();
        let writer = CacheWriter::new(workspace_root);
        writer.save_full(&original, &HashMap::new()).unwrap();

        let restored = Index::new();
        let loader = CacheLoader::new(workspace_root);
        loader.load(&restored, &HashSet::new()).unwrap();

        // デフォルトに戻る
        assert_eq!(
            restored.interpolate.resolved(),
            ("{{".to_string(), "}}".to_string())
        );
    }
}
