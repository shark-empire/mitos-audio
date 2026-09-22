use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("malformed response: {0}")]
    Protocol(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// The daemon answered with an error response.
    #[error("mitos-audio error [{code}]: {message}")]
    Daemon { code: String, message: String },

    #[error("connection closed by daemon")]
    Disconnected,
}