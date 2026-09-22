use std::collections::HashMap;

use serde_json::{json, Value};
use tokio::sync::{broadcast, Mutex};

use crate::config::AudioConfig;
use crate::devices::device::{Bus, Device, DeviceKind, DeviceState, Direction};
use crate::errors::AudioError;
use crate::ipc::messages::{Command, Event};
use crate::streams::stream::{AudioStream, StreamKind};

const KNOWN_PROFILES: &[&str] = &[
    "stereo", "headphones", "headset", "hdmi-stereo", "surround-5.1",
    "surround-7.1", "bluetooth-music", "bluetooth-headset", "usb-dac",
];

/// Central audio state + command dispatch.
///
/// v0.1 keeps state in memory, seeded with a demo backend.
/// `src/backend/alsa.rs` will later sync this state with real hardware —
/// the IPC surface stays identical either way.
pub struct AudioManager {
    config: AudioConfig,
    state: Mutex<AudioState>,
    events: broadcast::Sender<Event>,
}

struct AudioState {
    devices: HashMap<String, Device>,
    streams: HashMap<String, AudioStream>,
    default_output: Option<String>,
    default_input: Option<String>,
    master_volume: u32,
    master_muted: bool,
    mic_muted: bool,
    mic_gain: i32,
    profile: String,
}

impl AudioManager {
    pub fn new(config: AudioConfig, events: broadcast::Sender<Event>) -> Self {
        // Demo backend seed — replaced by real ALSA enumeration later.
        let mut devices = HashMap::new();
        for d in [
            Device::new("speakers", "Internal Speakers", DeviceKind::Speakers, Direction::Output, Bus::Internal),
            Device::new("headphones", "Analog Headphones", DeviceKind::Headphones, Direction::Output, Bus::Internal),
            Device::new("hdmi0", "HDMI 0", DeviceKind::Hdmi, Direction::Output, Bus::Hdmi),
            Device::new("usb-mic", "USB Microphone", DeviceKind::Microphone, Direction::Input, Bus::Usb),
            Device::new("bt-headset", "Bluetooth Headset", DeviceKind::Headset, Direction::Both, Bus::Bluetooth),
        ] {
            devices.insert(d.id.clone(), d);
        }
        if let Some(speakers) = devices.get_mut("speakers") {
            speakers.state = DeviceState::Active;
        }

        let mut streams = HashMap::new();
        streams.insert(
            "s-1".to_string(),
            AudioStream::new("s-1", "mitos-music", StreamKind::Playback, "speakers"),
        );

        let state = AudioState {
            devices,
            streams,
            default_output: Some("speakers".into()),
            default_input: Some("usb-mic".into()),
            master_volume: 50,
            master_muted: false,
            mic_muted: false,
            mic_gain: 0,
            profile: "stereo".into(),
        };

        Self { config, state: Mutex::new(state), events }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    pub async fn handle(&self, command: Command) -> Result<Value, AudioError> {
        match command {
            Command::Ping => Ok(json!({ "pong": true })),

            Command::GetState => {
                let s = self.state.lock().await;
                Ok(snapshot(&s))
            }

            Command::GetDefaults => {
                let s = self.state.lock().await;
                Ok(json!({ "default_output": s.default_output, "default_input": s.default_input }))
            }

            Command::ListDevices => {
                let s = self.state.lock().await;
                Ok(json!({
                    "devices": sorted_devices(&s),
                    "default_output": s.default_output,
                    "default_input": s.default_input,
                }))
            }

            Command::GetDevice { id } => {
                let s = self.state.lock().await;
                let d = s.devices.get(&id).ok_or_else(|| AudioError::DeviceNotFound(id))?;
                Ok(json!(d))
            }

            Command::SetDefaultOutput { id } => {
                {
                    let mut s = self.state.lock().await;
                    let d = s.devices.get(&id)
                        .ok_or_else(|| AudioError::DeviceNotFound(id.clone()))?;
                    if d.direction == Direction::Input {
                        return Err(AudioError::NotAnOutput(id));
                    }
                    s.default_output = Some(id.clone());
                }
                self.emit(Event::DefaultOutputChanged { id: id.clone() });
                Ok(json!({ "default_output": id }))
            }

            Command::SetDefaultInput { id } => {
                {
                    let mut s = self.state.lock().await;
                    let d = s.devices.get(&id)
                        .ok_or_else(|| AudioError::DeviceNotFound(id.clone()))?;
                    if d.direction == Direction::Output {
                        return Err(AudioError::NotAnInput(id));
                    }
                    s.default_input = Some(id.clone());
                }
                self.emit(Event::DefaultInputChanged { id: id.clone() });
                Ok(json!({ "default_input": id }))
            }

            Command::GetVolume { device } => {
                let s = self.state.lock().await;
                match device {
                    Some(id) => {
                        let d = s.devices.get(&id)
                            .ok_or_else(|| AudioError::DeviceNotFound(id))?;
                        Ok(json!({ "device": d.id, "volume": d.volume, "muted": d.muted }))
                    }
                    None => Ok(json!({ "volume": s.master_volume, "muted": s.master_muted })),
                }
            }

            Command::SetVolume { device, volume } => {
                if volume > self.config.max_volume {
                    return Err(AudioError::InvalidVolume(volume));
                }
                match device {
                    Some(id) => {
                        {
                            let mut s = self.state.lock().await;
                            let d = s.devices.get_mut(&id)
                                .ok_or_else(|| AudioError::DeviceNotFound(id.clone()))?;
                            d.volume = volume;
                        }
                        self.emit(Event::VolumeChanged { device: Some(id.clone()), volume });
                        Ok(json!({ "device": id, "volume": volume }))
                    }
                    None => {
                        {
                            let mut s = self.state.lock().await;
                            s.master_volume = volume;
                        }
                        self.emit(Event::VolumeChanged { device: None, volume });
                        Ok(json!({ "volume": volume }))
                    }
                }
            }

            Command::Mute => self.set_master_muted(true).await,
            Command::Unmute => self.set_master_muted(false).await,

            Command::ListStreams => {
                let s = self.state.lock().await;
                let mut streams: Vec<&AudioStream> = s.streams.values().collect();
                streams.sort_by(|a, b| a.id.cmp(&b.id));
                Ok(json!({ "streams": streams }))
            }

            Command::SetStreamVolume { stream_id, volume } => {
                if volume > self.config.max_volume {
                    return Err(AudioError::InvalidVolume(volume));
                }
                {
                    let mut s = self.state.lock().await;
                    let st = s.streams.get_mut(&stream_id)
                        .ok_or_else(|| AudioError::StreamNotFound(stream_id.clone()))?;
                    st.volume = volume;
                }
                self.emit(Event::StreamChanged { id: stream_id.clone() });
                Ok(json!({ "stream_id": stream_id, "volume": volume }))
            }

            Command::SetStreamMute { stream_id, mute } => {
                {
                    let mut s = self.state.lock().await;
                    let st = s.streams.get_mut(&stream_id)
                        .ok_or_else(|| AudioError::StreamNotFound(stream_id.clone()))?;
                    st.muted = mute;
                }
                self.emit(Event::StreamChanged { id: stream_id.clone() });
                Ok(json!({ "stream_id": stream_id, "muted": mute }))
            }

            Command::MoveStream { stream_id, device_id } => {
                {
                    let mut s = self.state.lock().await;
                    if !s.devices.contains_key(&device_id) {
                        return Err(AudioError::DeviceNotFound(device_id));
                    }
                    let st = s.streams.get_mut(&stream_id)
                        .ok_or_else(|| AudioError::StreamNotFound(stream_id.clone()))?;
                    st.device = device_id.clone();
                }
                self.emit(Event::StreamChanged { id: stream_id.clone() });
                Ok(json!({ "stream_id": stream_id, "device": device_id }))
            }

            Command::ListProfiles => {
                let s = self.state.lock().await;
                Ok(json!({ "profiles": KNOWN_PROFILES, "active": s.profile }))
            }

            Command::SetProfile { profile } => {
                if !KNOWN_PROFILES.contains(&profile.as_str()) {
                    return Err(AudioError::InvalidProfile(profile));
                }
                {
                    let mut s = self.state.lock().await;
                    s.profile = profile.clone();
                }
                self.emit(Event::ProfileChanged { profile: profile.clone() });
                Ok(json!({ "profile": profile }))
            }

            Command::GetMicrophone => {
                let s = self.state.lock().await;
                let device = s.default_input.as_ref()
                    .and_then(|id| s.devices.get(id)).cloned();
                Ok(json!({ "device": device, "muted": s.mic_muted, "gain_db": s.mic_gain }))
            }

            Command::SetMicrophoneGain { gain } => {
                let clamped = gain.clamp(-30, 30);
                let muted = {
                    let mut s = self.state.lock().await;
                    s.mic_gain = clamped;
                    s.mic_muted
                };
                self.emit(Event::MicrophoneChanged { muted, gain: clamped });
                Ok(json!({ "gain_db": clamped }))
            }

            Command::MuteMicrophone { mute } => {
                let gain = {
                    let mut s = self.state.lock().await;
                    s.mic_muted = mute;
                    s.mic_gain
                };
                self.emit(Event::MicrophoneChanged { muted: mute, gain });
                Ok(json!({ "muted": mute }))
            }

            Command::GetLevels => {
                let s = self.state.lock().await;
                Ok(json!({
                    "master_volume": s.master_volume,
                    "muted": s.master_muted,
                    // Real metering arrives with the ALSA backend; zeros until then.
                    "output_level": 0.0,
                    "input_level": 0.0,
                    "peak": 0.0,
                    "clipping": false,
                }))
            }

            // Normally intercepted by the connection handler (needs the socket).
            Command::SubscribeEvents => Ok(json!({ "subscribed": true })),
        }
    }

    async fn set_master_muted(&self, muted: bool) -> Result<Value, AudioError> {
        {
            let mut s = self.state.lock().await;
            s.master_muted = muted;
        }
        self.emit(Event::MuteChanged { muted });
        Ok(json!({ "muted": muted }))
    }
}

fn sorted_devices(s: &AudioState) -> Vec<&Device> {
    let mut devices: Vec<&Device> = s.devices.values().collect();
    devices.sort_by(|a, b| a.id.cmp(&b.id));
    devices
}

fn snapshot(s: &AudioState) -> Value {
    let devices = sorted_devices(s);
    let mut streams: Vec<&AudioStream> = s.streams.values().collect();
    streams.sort_by(|a, b| a.id.cmp(&b.id));
    json!({
        "service": "mitos-audio",
        "version": env!("CARGO_PKG_VERSION"),
        "devices": devices,
        "streams": streams,
        "default_output": s.default_output,
        "default_input": s.default_input,
        "master_volume": s.master_volume,
        "master_muted": s.master_muted,
        "microphone": { "muted": s.mic_muted, "gain_db": s.mic_gain },
        "profile": s.profile,
    })
}