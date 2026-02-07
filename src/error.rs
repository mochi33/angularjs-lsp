use std::io;

use thiserror::Error;

/// アナライザー解析エラー
#[derive(Debug, Error)]
pub enum AnalyzerError {
    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Tree-sitter error in {file} at line {line}")]
    TreeSitter { file: String, line: u32 },
}

/// キャッシュ操作エラー
#[derive(Debug, Error)]
pub enum CacheError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Deserialization error: {0}")]
    Deserialize(String),

    #[error("Cache version mismatch")]
    VersionMismatch,

    #[error("Cache not found")]
    NotFound,
}

/// サーバー全体のエラー
#[derive(Debug, Error)]
pub enum ServerError {
    #[error("Analyzer error: {0}")]
    Analyzer(#[from] AnalyzerError),

    #[error("Cache error: {0}")]
    Cache(#[from] CacheError),
}
