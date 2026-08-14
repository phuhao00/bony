use thiserror::Error;

#[derive(Debug, Error)]
pub enum EconomyError {
    #[error("{0}")]
    InvalidParams(String),
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Serialize(String),
    #[error("chain broken at line {line}: {reason}")]
    ChainBroken { line: usize, reason: String },
}

impl EconomyError {
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::InvalidParams(msg.into())
    }

    pub fn io(action: &str, path: &std::path::Path, e: impl std::fmt::Display) -> Self {
        Self::Io(format!("{action} {}: {e}", path.display()))
    }
}
