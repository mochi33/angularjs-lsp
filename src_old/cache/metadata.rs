use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// キャッシュフォーマットのバージョン
/// v2: HTMLキャッシュ対応
pub const CACHE_VERSION: u32 = 2;

/// キャッシュメタデータ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMetadata {
    /// キャッシュフォーマットバージョン
    pub version: u32,
    /// ツールバージョン
    pub tool_version: String,
    /// ファイルごとのメタデータ
    pub files: HashMap<String, FileMetadata>,
}

/// ファイルのメタデータ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    /// 最終更新時刻（Unix timestamp）
    pub mtime: u64,
    /// ファイルサイズ
    pub size: u64,
}

impl CacheMetadata {
    /// 新しいCacheMetadataを作成
    pub fn new() -> Self {
        Self {
            version: CACHE_VERSION,
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            files: HashMap::new(),
        }
    }

    /// バージョンが互換かどうかをチェック
    pub fn is_compatible(&self) -> bool {
        self.version == CACHE_VERSION && self.tool_version == env!("CARGO_PKG_VERSION")
    }
}

impl Default for CacheMetadata {
    fn default() -> Self {
        Self::new()
    }
}
