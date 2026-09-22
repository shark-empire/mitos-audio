use thiserror::Error;

/// All errors surfaced by mitos-audio.
#[derive(Debug, Error)]
pub enum AudioError {
    #[error("device not found: {0}")]
    DeviceNotFound(String),

    #[error("stream not found: {0}")]
    StreamNotFound(String),

    #[error("invalid volume: {0}")]
    InvalidVolume(u32),

    #[error("unknown profile: {0}")]
    InvalidProfile(String),

    #[error("device {0} is not an output")]
    NotAnOutput(String),

    #[error("device {0} is not an input")]
    NotAnInput(String),

    #[error("ipc error: {0}")]
    Ipc(String),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("configuration parse error: {0}")]
    ConfigParse(#[from] toml::de::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl AudioError {
    /// Stable error code used in IPC responses.
    pub fn code(&self) -> &'static str {
        match self {
            Self::DeviceNotFound(_) => "device-not-found",
            Self::StreamNotFound(_) => "stream-not-found",
            Self::InvalidVolume(_) => "invalid-volume",
            Self::InvalidProfile(_) => "invalid-profile",
            Self::NotAnOutput(_) => "not-an-output",
            Self::NotAnInput(_) => "not-an-input",
            Self::Ipc(_) => "ipc-error",
            Self::Config(_) => "config-error",
            Self::ConfigParse(_) => "config-parse-error",
            Self::Io(_) => "io-error",
            Self::Serialization(_) => "serialization-error",
        }
    }
}