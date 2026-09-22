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
    GetLevels,
    SubscribeEvents,
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
            "ListProfiles" => Command::ListProfiles,
            "SetProfile" => Command::SetProfile { profile: req(&self.params, "profile")? },
            "GetMicrophone" => Command::GetMicrophone,
            "SetMicrophoneGain" => Command::SetMicrophoneGain { gain: req(&self.params, "gain")? },
            "MuteMicrophone" => Command::MuteMicrophone { mute: req(&self.params, "mute")? },
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
    MicrophoneChanged { muted: bool, gain: i32 },
}