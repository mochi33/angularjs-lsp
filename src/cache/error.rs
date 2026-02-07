use thiserror::Error;

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Deserialize error: {0}")]
    Deserialize(String),

    #[error("Cache version mismatch")]
    VersionMismatch,

    #[error("Cache not found")]
    NotFound,
}

impl From<bincode::Error> for CacheError {
    fn from(e: bincode::Error) -> Self {
        CacheError::Deserialize(e.to_string())
    }
}
