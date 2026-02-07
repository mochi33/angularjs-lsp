use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Cache format version
/// v2: HTML cache support
pub const CACHE_VERSION: u32 = 2;

/// Cache metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMetadata {
    pub version: u32,
    pub tool_version: String,
    pub files: HashMap<String, FileMetadata>,
}

/// File metadata for cache validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    pub mtime: u64,
    pub size: u64,
}

impl CacheMetadata {
    pub fn new() -> Self {
        Self {
            version: CACHE_VERSION,
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            files: HashMap::new(),
        }
    }

    pub fn is_compatible(&self) -> bool {
        self.version == CACHE_VERSION && self.tool_version == env!("CARGO_PKG_VERSION")
    }
}

impl Default for CacheMetadata {
    fn default() -> Self {
        Self::new()
    }
}
