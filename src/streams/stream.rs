use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamKind {
    Playback,
    Recording,
    Monitoring,
    Capture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamState {
    Idle,
    Running,
    Suspended,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioStream {
    pub id: String,
    pub application: String,
    pub kind: StreamKind,
    pub device: String,
    pub volume: u32,
    pub muted: bool,
    pub sample_rate: u32,
    pub channels: u32,
    pub format: String,
    pub state: StreamState,
    #[serde(default)]
    pub follows_default: bool,

}

impl AudioStream {
    pub fn new(id: &str, application: &str, kind: StreamKind, device: &str) -> Self {
        Self {
            id: id.to_string(),
            application: application.to_string(),
            kind,
            device: device.to_string(),
            volume: 100,
            muted: false,
            sample_rate: 48000,
            channels: 2,
            format: "s16le".to_string(),
            state: StreamState::Running,
            follows_default: false,
        }
    }

    pub fn direction(&self) -> &'static str {
        match self.kind {
            StreamKind::Playback => "output",
            StreamKind::Recording | StreamKind::Capture => "input",
            StreamKind::Monitoring => "loopback",
        }
    }
}