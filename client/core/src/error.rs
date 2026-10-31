use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// The client token is missing, expired or revoked.
    #[error("Key 无效或已被撤销")]
    Unauthorized,
    #[error("{message}")]
    Api { status: u16, message: String },
    #[error("无法连接服务器：{0}")]
    Network(String),
    #[error("读写 {path} 失败：{message}")]
    File { path: String, message: String },
    #[error("{0} 格式无效，请先手动修复")]
    InvalidConfig(String),
    #[error("{0}")]
    Other(String),
}

impl From<reqwest::Error> for CoreError {
    fn from(error: reqwest::Error) -> Self {
        // reqwest errors can carry the URL but never headers, so no secret leaks.
        CoreError::Network(error.without_url().to_string())
    }
}

impl CoreError {
    pub fn file(path: &std::path::Path, error: impl std::fmt::Display) -> Self {
        CoreError::File {
            path: path.display().to_string(),
            message: error.to_string(),
        }
    }
}

/// Tauri commands return errors to the UI as `{ kind, message }`.
impl Serialize for CoreError {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let kind = match self {
            CoreError::Unauthorized => "unauthorized",
            CoreError::Api { .. } => "api",
            CoreError::Network(_) => "network",
            CoreError::File { .. } => "file",
            CoreError::InvalidConfig(_) => "invalid_config",
            CoreError::Other(_) => "other",
        };
        let mut value = serializer.serialize_struct("CoreError", 2)?;
        value.serialize_field("kind", kind)?;
        value.serialize_field("message", &self.to_string())?;
        value.end()
    }
}

pub type Result<T> = std::result::Result<T, CoreError>;
