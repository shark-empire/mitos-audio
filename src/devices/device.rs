use serde::{Deserialize, Serialize};

/// Physical or virtual audio device classes known to the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Speakers,
    Headphones,
    Headset,
    Hdmi,
    DisplayPort,
    Usb,
    Bluetooth,
    Microphone,
    Virtual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Input,
    Output,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceState {
    Disconnected,
    Available,
    Active,
    Suspended,
    Unavailable,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Bus {
    Internal,
    Usb,
    Bluetooth,
    Hdmi,
    Virtual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: DeviceKind,
    pub direction: Direction,
    pub bus: Bus,
    pub state: DeviceState,
    pub profiles: Vec<String>,
    pub active_profile: Option<String>,
    pub channels: u32,
    pub sample_rates: Vec<u32>,
    pub volume: u32,
    pub muted: bool,
    /// ALSA device identifier (e.g. "hw:0,0") — filled by the backend.
    pub alsa: Option<String>,
}

impl Device {
    pub fn new(id: &str, name: &str, kind: DeviceKind, direction: Direction, bus: Bus) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            description: name.to_string(),
            kind,
            direction,
            bus,
            state: DeviceState::Available,
            profiles: vec!["stereo".to_string()],
            active_profile: Some("stereo".to_string()),
            channels: 2,
            sample_rates: vec![44100, 48000],
            volume: 50,
            muted: false,
            alsa: None,
        }
    }
}