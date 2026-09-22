//! Typed mirrors of the daemon's wire model (see docs/ipc.md).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Speakers, Headphones, Headset, Hdmi, DisplayPort,
    Usb, Bluetooth, Microphone, Virtual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction { Input, Output, Both }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceState {
    Disconnected, Available, Active, Suspended, Unavailable, Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Bus { Internal, Usb, Bluetooth, Hdmi, Virtual }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: DeviceKind,
    pub direction: Direction,
    pub bus: Bus,
    pub state: DeviceState,
    pub profiles: Vec<String>,
    #[serde(default)]
    pub active_profile: Option<String>,
    pub channels: u32,
    pub sample_rates: Vec<u32>,
    pub volume: u32,
    pub muted: bool,
    /// ALSA device id (e.g. "hw:0,0") when backed by real hardware.
    #[serde(default)]
    pub alsa: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamKind { Playback, Recording, Monitoring, Capture }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamState { Idle, Running, Suspended, Stopped }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamInfo {
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

/// Result of `ListDevices` / `Rescan`.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceList {
    pub devices: Vec<DeviceInfo>,
    #[serde(default)]
    pub default_output: Option<String>,
    #[serde(default)]
    pub default_input: Option<String>,
}

/// Master/device volume as reported by `GetVolume`.
#[derive(Debug, Clone, Deserialize)]
pub struct VolumeInfo {
    #[serde(default)]
    pub device: Option<String>,
    pub volume: u32,
    pub muted: bool,
}

/// Result of `SetVolume` — `hw_applied` is false when the device has no
/// hardware volume control (state still updated).
#[derive(Debug, Clone, Deserialize)]
pub struct VolumeChangeResult {
    #[serde(default)]
    pub device: Option<String>,
    pub volume: u32,
    #[serde(default)]
    pub hw_applied: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProfileList {
    pub profiles: Vec<String>,
    pub active: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MicrophoneSummary {
    pub muted: bool,
    #[serde(rename = "gain_db")]
    pub gain_db: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MicrophoneStatus {
    #[serde(default)]
    pub device: Option<DeviceInfo>,
    pub muted: bool,
    #[serde(rename = "gain_db")]
    pub gain_db: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Levels {
    pub master_volume: u32,
    pub muted: bool,
    pub output_level: f32,
    pub input_level: f32,
    pub peak: f32,
    pub clipping: bool,
}

/// One level frame from `SubscribeLevels` / `GetLevels`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct LevelFrame {
    pub output_level: f32,
    pub input_level: f32,
    pub peak: f32,
    pub clipping: bool,
}

/// Full daemon snapshot (`GetState`).
#[derive(Debug, Clone, Deserialize)]
pub struct SystemState {
    pub service: String,
    pub version: String,
    #[serde(default)]
    pub backend: String,
    pub devices: Vec<DeviceInfo>,
    pub streams: Vec<StreamInfo>,
    #[serde(default)]
    pub default_output: Option<String>,
    #[serde(default)]
    pub default_input: Option<String>,
    pub master_volume: u32,
    pub master_muted: bool,
    pub microphone: MicrophoneSummary,
    pub profile: String,
    #[serde(default)]
    pub data_socket: Option<String>,
}