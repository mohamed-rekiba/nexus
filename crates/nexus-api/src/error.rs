use serde::{Deserialize, Serialize};

/// An error a plugin returns, or the host reports about a plugin call.
///
/// Every variant carries text only.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum PluginError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("not supported: {0}")]
    Unsupported(String),
    /// Something must happen first, for example installing an emulator version.
    #[error("precondition failed: {0}")]
    Precondition(String),
    #[error("plugin call timed out after {0} ms")]
    Timeout(u64),
    #[error("plugin panicked: {0}")]
    Panicked(String),
    #[error("{0}")]
    Other(String),
}

pub type PluginResult<T> = Result<T, PluginError>;

impl From<crate::InvalidPath> for PluginError {
    fn from(e: crate::InvalidPath) -> Self {
        Self::InvalidInput(e.to_string())
    }
}

impl From<crate::InvalidId> for PluginError {
    fn from(e: crate::InvalidId) -> Self {
        Self::InvalidInput(e.to_string())
    }
}

impl From<serde_json::Error> for PluginError {
    fn from(e: serde_json::Error) -> Self {
        Self::InvalidInput(e.to_string())
    }
}
