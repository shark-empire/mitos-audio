//! Typed events, plus the `Resync` pattern that keeps GUIs correct.

use serde_json::Value;

/// Events delivered by [`crate::AudioClient::subscribe`].
#[derive(Debug, Clone)]
pub enum ClientEvent {
    DeviceAdded { id: String, name: String },
    DeviceRemoved { id: String },
    DeviceChanged { id: String },
    DefaultOutputChanged { id: String },
    DefaultInputChanged { id: String },
    VolumeChanged { device: Option<String>, volume: u32 },
    MuteChanged { muted: bool },
    StreamAdded { id: String, application: String },
    StreamRemoved { id: String },
    StreamChanged { id: String },
    ProfileChanged { profile: String },
    MicrophoneChanged {
        muted: bool,
        gain: i32,
        noise_suppression: bool,
        echo_cancellation: bool,
        agc: bool,
    },
    EffectsChanged {
        device: String,
        enabled: bool,
        preset: String,
        bands: Vec<f32>,
    },

    /// **Synthetic** (not from the daemon): the monitor connected or
    /// reconnected. Fetch fresh state with `get_state()` when you see this —
    /// you may have missed events while disconnected.
    Resync,

    /// Unknown event from a newer daemon. Ignore it (compatibility rule:
    /// clients must tolerate unknown events).
    Unknown { name: String },
}

impl ClientEvent {
    /// Parse one wire line: `{"event":"VolumeChanged","data":{...}}`.
    /// Returns `None` for lines that are not events (responses, garbage).
    pub(crate) fn from_wire(value: &Value) -> Option<Self> {
        let name = value.get("event")?.as_str()?;
        let data = value.get("data");
        let field = |key: &str| data.and_then(|d| d.get(key));
        let as_str = |key: &str| field(key).and_then(Value::as_str).map(str::to_owned);
        let as_u32 = |key: &str| field(key).and_then(Value::as_u64).map(|v| v as u32);

        Some(match name {
            "DeviceAdded" => {
                ClientEvent::DeviceAdded { id: as_str("id")?, name: as_str("name")? }
            }
            "DeviceRemoved" => ClientEvent::DeviceRemoved { id: as_str("id")? },
            "DeviceChanged" => ClientEvent::DeviceChanged { id: as_str("id")? },
            "DefaultOutputChanged" => ClientEvent::DefaultOutputChanged { id: as_str("id")? },
            "DefaultInputChanged" => ClientEvent::DefaultInputChanged { id: as_str("id")? },
            "VolumeChanged" => ClientEvent::VolumeChanged {
                device: field("device").and_then(Value::as_str).map(str::to_owned),
                volume: as_u32("volume")?,
            },
            "MuteChanged" => {
                ClientEvent::MuteChanged { muted: field("muted").and_then(Value::as_bool)? }
            }
            "StreamAdded" => ClientEvent::StreamAdded {
                id: as_str("id")?,
                application: as_str("application")?,
            },
            "StreamRemoved" => ClientEvent::StreamRemoved { id: as_str("id")? },
            "StreamChanged" => ClientEvent::StreamChanged { id: as_str("id")? },
            "ProfileChanged" => ClientEvent::ProfileChanged { profile: as_str("profile")? },
            "MicrophoneChanged" => ClientEvent::MicrophoneChanged {
                muted: field("muted").and_then(Value::as_bool)?,
                gain: field("gain").and_then(Value::as_i64)? as i32,
                // Defaulted, not `?`-required: a daemon predating these
                // toggles won't send them, and that's not a parse failure.
                noise_suppression: field("noise_suppression").and_then(Value::as_bool).unwrap_or(false),
                echo_cancellation: field("echo_cancellation").and_then(Value::as_bool).unwrap_or(false),
                agc: field("agc").and_then(Value::as_bool).unwrap_or(false),
            },
            "EffectsChanged" => ClientEvent::EffectsChanged {
                device: as_str("device")?,
                enabled: field("enabled").and_then(Value::as_bool)?,
                preset: as_str("preset")?,
                bands: field("bands")
                    .and_then(Value::as_array)
                    .map(|arr| arr.iter().filter_map(Value::as_f64).map(|v| v as f32).collect())
                    .unwrap_or_default(),
            },
            other => ClientEvent::Unknown { name: other.to_string() },
        })
    }
}

/// Receive half of the background event monitor.
pub struct EventStream {
    rx: tokio::sync::mpsc::Receiver<ClientEvent>,
}

impl EventStream {
    /// Await the next event. Returns `None` once the client is dropped
    /// (all clones) — or never, since the monitor reconnects forever.
    pub async fn recv(&mut self) -> Option<ClientEvent> {
        self.rx.recv().await
    }
}