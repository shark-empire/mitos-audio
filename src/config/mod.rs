use serde::{Deserialize, Serialize};

use crate::errors::AudioError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    pub socket_path: String,
    /// Backend selection: "auto" (default) | "alsa" | "demo"
    pub backend: String,
    pub default_sample_rate: u32,
    pub default_channels: u32,
    pub volume_step: u32,
    pub max_volume: u32,
    pub log_level: String,

    // ── persistence ──
    pub persist: bool,
    pub state_path: String,
    pub routing_path: String,

    // ── hotplug ──
    pub hotplug: HotplugConfig,

    // ── level metering ──
    pub metering: MeteringConfig,
    
    /// Data-plane socket (binary audio frames). When left at the default
    /// and socket_path is non-default, the daemon derives it.
    pub data_socket_path: String,
    /// Playback buffer latency (µs = ms × 1000) per output sink.
    pub playback_latency_ms: u32,

}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HotplugConfig {
    /// Use the udev watcher (instant hotplug) when built with the feature.
    pub udev: bool,
    /// Periodic rescan interval in seconds — catches external mixer changes
    /// (alsamixer) and acts as a udev safety net. 0 disables.
    pub poll_secs: u64,
}

impl Default for HotplugConfig {
    fn default() -> Self {
        Self { udev: true, poll_secs: 3 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MeteringConfig {
    pub enabled: bool,
    /// LevelChanged push interval (ms), clamped to 20–1000.
    pub interval_ms: u64,
    /// Meter the default capture PCM on real ALSA (opens `default`).
    pub capture_input: bool,
}

impl Default for MeteringConfig {
    fn default() -> Self {
        Self { enabled: true, interval_ms: 100, capture_input: true }
    }
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
            persist: true,
            state_path: "/var/lib/mitos/audio/state.json".to_string(),
            routing_path: "/etc/mitos/routing.toml".to_string(),
            hotplug: HotplugConfig::default(),
            metering: MeteringConfig::default(),
            data_socket_path: "/run/mitos/audio-data.sock".to_string(),
            playback_latency_ms: 50,
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