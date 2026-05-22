use thiserror::Error;

#[derive(Debug, Error)]
pub enum MemoryTreeError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("summarizer error: {0}")]
    Summarizer(String),
}

pub type Result<T> = std::result::Result<T, MemoryTreeError>;
