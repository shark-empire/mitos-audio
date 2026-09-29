use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::AudioError;

/// A request received from a client.
#[derive(Debug, Deserialize)]
pub struct Request {
    pub id: u64,
    pub command: String,
    #[serde(default)]
    pub params: Value,
}

/// Typed commands (matches the roadmap IPC surface, plus a few getters).
#[derive(Debug)]
pub enum Command {
    Ping,
    GetState,
    GetDefaults,
    ListDevices,
    GetDevice { id: String },
    SetDefaultOutput { id: String },
    SetDefaultInput { id: String },
    GetVolume { device: Option<String> },
    SetVolume { device: Option<String>, volume: u32 },
    Mute,
    Unmute,
    ListStreams,
    SetStreamVolume { stream_id: String, volume: u32 },
    SetStreamMute { stream_id: String, mute: bool },
    MoveStream { stream_id: String, device_id: String },
    ListProfiles,
    SetProfile { profile: String },
    GetMicrophone,
    SetMicrophoneGain { gain: i32 },
    MuteMicrophone { mute: bool },
    /// Any field left `None` leaves that toggle unchanged — a partial
    /// update, not a full replace (matches how `SetVolume`'s `device` is
    /// optional rather than every setter needing every field).
    SetMicrophoneProcessing {
        noise_suppression: Option<bool>,
        echo_cancellation: Option<bool>,
        agc: Option<bool>,
    },
    GetLevels,
    SubscribeEvents,
    Rescan,
    SubscribeLevels,
    CreateStream { application: String, device: Option<String>, kind: Option<String> },
    DestroyStream { stream_id: String },
    ReloadRouting,
    ReloadPolicy,
    GetEffects { device: Option<String> },
    SetEffectsEnabled { device: Option<String>, enabled: bool },
    SetEffectsPreset { device: Option<String>, preset: String },
    /// `bands` must have exactly 10 values (dB), one per
    /// `effects::equalizer::BAND_FREQUENCIES` entry.
    SetEqualizerBands { device: Option<String>, bands: Vec<f32> },
    /// Synchronized multi-speaker output — see `crate::groups`.
    CreateGroup { name: String, members: Vec<String> },
    ListGroups,
    DeleteGroup { id: String },
    /// `latency_ms` omitted defaults to 0 — see `crate::groups::compute_delays_ms`.
    AddGroupMember { id: String, device_id: String, latency_ms: Option<u32> },
    RemoveGroupMember { id: String, device_id: String },
    SetGroupMemberLatency { id: String, device_id: String, latency_ms: u32 },
}

impl Request {
    /// Decode `command` + `params` into a typed [`Command`].
    pub fn command(&self) -> Result<Command, AudioError> {
        fn req<T: serde::de::DeserializeOwned>(params: &Value, key: &str) -> Result<T, AudioError> {
            let v = params.get(key).cloned().unwrap_or(Value::Null);
            serde_json::from_value(v)
                .map_err(|e| AudioError::Ipc(format!("invalid parameter '{key}': {e}")))
        }
        fn opt<T: serde::de::DeserializeOwned>(
            params: &Value,
            key: &str,
        ) -> Result<Option<T>, AudioError> {
            match params.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => serde_json::from_value(v.clone())
                    .map(Some)
                    .map_err(|e| AudioError::Ipc(format!("invalid parameter '{key}': {e}"))),
            }
        }

        Ok(match self.command.as_str() {
            "Ping" => Command::Ping,
            "GetState" => Command::GetState,
            "GetDefaults" => Command::GetDefaults,
            "ListDevices" => Command::ListDevices,
            "GetDevice" => Command::GetDevice { id: req(&self.params, "id")? },
            "SetDefaultOutput" => Command::SetDefaultOutput { id: req(&self.params, "id")? },
            "SetDefaultInput" => Command::SetDefaultInput { id: req(&self.params, "id")? },
            "GetVolume" => Command::GetVolume { device: opt(&self.params, "device")? },
            "Rescan" => Command::Rescan,
            "SetVolume" => Command::SetVolume {
                device: opt(&self.params, "device")?,
                volume: req(&self.params, "volume")?,
            },
            "Mute" => Command::Mute,
            "Unmute" => Command::Unmute,
            "ListStreams" => Command::ListStreams,
            "SetStreamVolume" => Command::SetStreamVolume {
                stream_id: req(&self.params, "stream_id")?,
                volume: req(&self.params, "volume")?,
            },
            "SetStreamMute" => Command::SetStreamMute {
                stream_id: req(&self.params, "stream_id")?,
                mute: req(&self.params, "mute")?,
            },
            "MoveStream" => Command::MoveStream {
                stream_id: req(&self.params, "stream_id")?,
                device_id: req(&self.params, "device_id")?,
            },
            "SubscribeLevels" => Command::SubscribeLevels,
            "CreateStream" => Command::CreateStream {
                application: req(&self.params, "application")?,
                device: opt(&self.params, "device")?,
                kind: opt(&self.params, "kind")?,
            },
            "DestroyStream" => Command::DestroyStream { stream_id: req(&self.params, "stream_id")? },
            "ReloadRouting" => Command::ReloadRouting,
            "ReloadPolicy" => Command::ReloadPolicy,
            "GetEffects" => Command::GetEffects { device: opt(&self.params, "device")? },
            "SetEffectsEnabled" => Command::SetEffectsEnabled {
                device: opt(&self.params, "device")?,
                enabled: req(&self.params, "enabled")?,
            },
            "SetEffectsPreset" => Command::SetEffectsPreset {
                device: opt(&self.params, "device")?,
                preset: req(&self.params, "preset")?,
            },
            "SetEqualizerBands" => Command::SetEqualizerBands {
                device: opt(&self.params, "device")?,
                bands: req(&self.params, "bands")?,
            },
            "CreateGroup" => Command::CreateGroup {
                name: req(&self.params, "name")?,
                members: req(&self.params, "members")?,
            },
            "ListGroups" => Command::ListGroups,
            "DeleteGroup" => Command::DeleteGroup { id: req(&self.params, "id")? },
            "AddGroupMember" => Command::AddGroupMember {
                id: req(&self.params, "id")?,
                device_id: req(&self.params, "device_id")?,
                latency_ms: opt(&self.params, "latency_ms")?,
            },
            "RemoveGroupMember" => Command::RemoveGroupMember {
                id: req(&self.params, "id")?,
                device_id: req(&self.params, "device_id")?,
            },
            "SetGroupMemberLatency" => Command::SetGroupMemberLatency {
                id: req(&self.params, "id")?,
                device_id: req(&self.params, "device_id")?,
                latency_ms: req(&self.params, "latency_ms")?,
            },
            "ListProfiles" => Command::ListProfiles,
            "SetProfile" => Command::SetProfile { profile: req(&self.params, "profile")? },
            "GetMicrophone" => Command::GetMicrophone,
            "SetMicrophoneGain" => Command::SetMicrophoneGain { gain: req(&self.params, "gain")? },
            "MuteMicrophone" => Command::MuteMicrophone { mute: req(&self.params, "mute")? },
            "SetMicrophoneProcessing" => Command::SetMicrophoneProcessing {
                noise_suppression: opt(&self.params, "noise_suppression")?,
                echo_cancellation: opt(&self.params, "echo_cancellation")?,
                agc: opt(&self.params, "agc")?,
            },
            "GetLevels" => Command::GetLevels,
            "SubscribeEvents" => Command::SubscribeEvents,
            other => return Err(AudioError::Ipc(format!("unknown command: {other}"))),
        })
    }
}

/// Response sent for every request.
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Self {
        Self { id, ok: true, result: Some(result), error: None }
    }

    pub fn from_error(id: u64, err: &AudioError) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(ErrorBody {
                code: err.code().to_string(),
                message: err.to_string(),
            }),
        }
    }
}

/// Events broadcast to subscribed clients.
/// Wire form: {"event":"VolumeChanged","data":{...}}
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "PascalCase")]
pub enum Event {
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
    /// Pushed at ~10 Hz to level subscribers while any exist (see
    /// `SubscribeLevels`). Never sent on the general `SubscribeEvents`
    /// channel.
    LevelChanged { output_level: f32, input_level: f32, peak: f32, clipping: bool },
    EffectsChanged { device: String, enabled: bool, preset: String, bands: Vec<f32> },
    GroupChanged { id: String },
    GroupRemoved { id: String },
}