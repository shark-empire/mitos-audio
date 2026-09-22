use serde::{Deserialize, Serialize};

use crate::errors::AudioError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    pub socket_path: String,
    pub backend: String,
    pub default_sample_rate: u32,
    pub default_channels: u32,
    pub volume_step: u32,
    pub max_volume: u32,
    pub log_level: String,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            socket_path: "/run/mitos/audio.sock".to_string(),
            backend: "auto".to_string(),
            default_sample_rate: 48000,
            default_channels: 2,
            volume_step: 5,
            max_volume: 100,
            log_level: "info".to_string(),
        
        }
    }
}

impl AudioConfig {
    /// Load from `path`; fall back to defaults if the file does not exist yet.
    pub fn load_or_default(path: &str) -> Result<Self, AudioError> {
        match std::fs::read_to_string(path) {
            Ok(contents) => Ok(toml::from_str(&contents)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!(path, "config not found, using defaults");
                Ok(Self::default())
            }
            Err(e) => Err(AudioError::Config(format!("cannot read {path}: {e}"))),
        }
    }
}