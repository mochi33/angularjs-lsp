use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::Url;
use tracing::{debug, info, warn};

use crate::index::{
    ControllerScope, HtmlControllerScope, HtmlDirectiveReference, HtmlFormBinding,
    HtmlLocalVariable, HtmlLocalVariableReference, HtmlScopeReference, NgIncludeBinding,
    Symbol, SymbolIndex, SymbolReference, TemplateBinding,
};

use super::metadata::{CacheMetadata, CACHE_VERSION};

/// キャッシュ検証結果
pub struct CacheValidation {
    /// キャッシュが有効なファイル（再解析不要）
    pub valid_files: HashSet<PathBuf>,
    /// キャッシュが無効なファイル（再解析必要）
    pub invalid_files: HashSet<PathBuf>,
}

/// キャッシュエラー
#[derive(Debug)]
pub enum CacheError {
    Io(std::io::Error),
    Deserialize(String),
    VersionMismatch,
    NotFound,
}

impl From<std::io::Error> for CacheError {
    fn from(e: std::io::Error) -> Self {
        CacheError::Io(e)
    }
}

impl From<bincode::Error> for CacheError {
    fn from(e: bincode::Error) -> Self {
        CacheError::Deserialize(e.to_string())
    }
}

/// キャッシュされたシンボルデータ
#[derive(serde::Serialize, serde::Deserialize)]
pub struct CachedSymbolData {
    /// ファイルURIの文字列表現
    pub uri: String,
    /// シンボル定義
    pub definitions: Vec<Symbol>,
    /// シンボル参照
    pub references: Vec<SymbolReference>,
    /// コントローラースコープ
    pub controller_scopes: Vec<ControllerScope>,
    /// HTML内のng-controllerスコープ
    #[serde(default)]
    pub html_controller_scopes: Vec<HtmlControllerScope>,
    /// HTML内のスコープ参照
    #[serde(default)]
    pub html_scope_references: Vec<HtmlScopeReference>,
    /// HTML内のローカル変数定義
    #[serde(default)]
    pub html_local_variables: Vec<HtmlLocalVariable>,
    /// HTML内のローカル変数参照
    #[serde(default)]
    pub html_local_variable_references: Vec<HtmlLocalVariableReference>,
    /// HTML内のフォームバインディング
    #[serde(default)]
    pub html_form_bindings: Vec<HtmlFormBinding>,
    /// HTML内のディレクティブ参照
    #[serde(default)]
    pub html_directive_references: Vec<HtmlDirectiveReference>,
}

/// キャッシュされたグローバルデータ（ファイルに依存しないデータ）
#[derive(serde::Serialize, serde::Deserialize)]
pub struct CachedGlobalData {
    /// テンプレートバインディング（template_path -> binding）
    pub template_bindings: Vec<TemplateBinding>,
    /// ng-includeバインディング
    pub ng_include_bindings: Vec<(String, NgIncludeBinding)>,
}

/// キャッシュローダー
pub struct CacheLoader {
    cache_dir: PathBuf,
}

impl CacheLoader {
    pub fn new(workspace_root: &Path) -> Self {
        Self {
            cache_dir: workspace_root.join(".angularjs-lsp/cache/v1"),
        }
    }

    /// キャッシュディレクトリのパスを取得
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// キャッシュの有効性を検証
    /// files: (path, mtime, size) のリスト
    pub fn validate(
        &self,
        files: &[(PathBuf, u64, u64)],
    ) -> Result<CacheValidation, CacheError> {
        let metadata_path = self.cache_dir.join("metadata.json");
        if !metadata_path.exists() {
            return Err(CacheError::NotFound);
        }

        let metadata_content = fs::read_to_string(&metadata_path)?;
        let metadata: CacheMetadata = serde_json::from_str(&metadata_content)
            .map_err(|e| CacheError::Deserialize(e.to_string()))?;

        // バージョン互換性チェック
        if !metadata.is_compatible() {
            warn!(
                "Cache version mismatch: {} (expected {})",
                metadata.version, CACHE_VERSION
            );
            return Err(CacheError::VersionMismatch);
        }

        let mut valid_files = HashSet::new();
        let mut invalid_files = HashSet::new();

        for (path, mtime, size) in files {
            let path_str = path.to_string_lossy().to_string();
            if let Some(cached_meta) = metadata.files.get(&path_str) {
                if cached_meta.mtime == *mtime && cached_meta.size == *size {
                    valid_files.insert(path.clone());
                } else {
                    debug!("Cache invalid for {}: mtime/size changed", path_str);
                    invalid_files.insert(path.clone());
                }
            } else {
                debug!("Cache miss for {}: not in cache", path_str);
                invalid_files.insert(path.clone());
            }
        }

        Ok(CacheValidation {
            valid_files,
            invalid_files,
        })
    }

    /// キャッシュからインデックスを復元
    pub fn load(
        &self,
        index: &SymbolIndex,
        valid_files: &HashSet<PathBuf>,
    ) -> Result<(), CacheError> {
        let data_path = self.cache_dir.join("symbols.bin");
        if !data_path.exists() {
            return Err(CacheError::NotFound);
        }

        let data = fs::read(&data_path)?;
        let cached_data: Vec<CachedSymbolData> = bincode::deserialize(&data)?;

        let total_entries = cached_data.len();
        let total_definitions: usize = cached_data.iter().map(|e| e.definitions.len()).sum();
        let total_references: usize = cached_data.iter().map(|e| e.references.len()).sum();
        let total_scopes: usize = cached_data.iter().map(|e| e.controller_scopes.len()).sum();
        let total_html_scopes: usize = cached_data.iter().map(|e| e.html_controller_scopes.len()).sum();
        let total_html_refs: usize = cached_data.iter().map(|e| e.html_scope_references.len()).sum();
        info!(
            "Cache contains {} entries, {} definitions, {} references, {} scopes, {} html_scopes, {} html_refs",
            total_entries, total_definitions, total_references, total_scopes, total_html_scopes, total_html_refs
        );

        let mut loaded_definitions = 0;
        let mut loaded_html_scopes = 0;
        let mut skipped_entries = 0;

        for entry in cached_data {
            // URIをパスに変換して有効性チェック
            if let Ok(uri) = Url::parse(&entry.uri) {
                if let Ok(path) = uri.to_file_path() {
                    if !valid_files.contains(&path) {
                        skipped_entries += 1;
                        continue;
                    }
                }
            }

            loaded_definitions += entry.definitions.len();
            loaded_html_scopes += entry.html_controller_scopes.len();

            // 定義を追加
            for symbol in entry.definitions {
                index.add_definition(symbol);
            }

            // 参照を追加
            for reference in entry.references {
                index.add_reference(reference);
            }

            // コントローラースコープを追加
            for scope in entry.controller_scopes {
                index.add_controller_scope(scope);
            }

            // HTML関連データを追加
            for scope in entry.html_controller_scopes {
                index.add_html_controller_scope(scope);
            }

            for reference in entry.html_scope_references {
                index.add_html_scope_reference(reference);
            }

            for variable in entry.html_local_variables {
                index.add_html_local_variable(variable);
            }

            for reference in entry.html_local_variable_references {
                index.add_html_local_variable_reference(reference);
            }

            for binding in entry.html_form_bindings {
                index.add_html_form_binding(binding);
            }

            for reference in entry.html_directive_references {
                index.add_html_directive_reference(reference);
            }
        }

        // グローバルデータを復元
        self.load_global_data(index)?;

        info!(
            "Loaded {} definitions, {} html_scopes from cache (skipped {} entries, valid_files: {})",
            loaded_definitions, loaded_html_scopes, skipped_entries, valid_files.len()
        );
        Ok(())
    }

    /// グローバルデータをロード
    fn load_global_data(&self, index: &SymbolIndex) -> Result<(), CacheError> {
        let global_path = self.cache_dir.join("global.bin");
        if !global_path.exists() {
            debug!("No global cache file found");
            return Ok(());
        }

        let data = fs::read(&global_path)?;
        let global_data: CachedGlobalData = bincode::deserialize(&data)?;

        // テンプレートバインディングを復元
        for binding in global_data.template_bindings {
            index.add_template_binding(binding);
        }

        // ng-includeバインディングを復元
        for (key, binding) in global_data.ng_include_bindings {
            index.add_ng_include_binding_with_key(key, binding);
        }

        info!(
            "Loaded global data from cache"
        );
        Ok(())
    }
}
