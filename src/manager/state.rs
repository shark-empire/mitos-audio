use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::{broadcast, Mutex};

use crate::backend::AudioBackend;
use crate::config::AudioConfig;
use crate::devices::device::{Device, DeviceKind, DeviceState, Direction};
use crate::errors::AudioError;
use crate::ipc::messages::{Command, Event};
use crate::streams::stream::{AudioStream, StreamKind};

const KNOWN_PROFILES: &[&str] = &[
    "stereo", "headphones", "headset", "hdmi-stereo", "surround-5.1",
    "surround-7.1", "bluetooth-music", "bluetooth-headset", "usb-dac",
];

/// Central audio state + command dispatch, backed by an [`AudioBackend`].
///
/// The backend owns *hardware truth* (which devices exist, their live
/// volume/mute); the manager owns policy (defaults, profiles, mic settings)
/// and is the only component that talks to the backend.
pub struct AudioManager {
    config: AudioConfig,
    backend: Arc<dyn AudioBackend>,
    backend_name: &'static str,
    state: Mutex<AudioState>,
    events: broadcast::Sender<Event>,
}

struct AudioState {
    devices: HashMap<String, Device>,
    streams: HashMap<String, AudioStream>,
    default_output: Option<String>,
    default_input: Option<String>,
    mic_muted: bool,
    mic_gain: i32,
    profile: String,
}

impl AudioManager {
    pub fn new(
        config: AudioConfig,
        backend: Arc<dyn AudioBackend>,
        events: broadcast::Sender<Event>,
    ) -> Self {
        // Demo streams exist until real application streams arrive (roadmap).
        let mut streams = HashMap::new();
        if backend.name() == "demo" {
            streams.insert(
                "s-1".to_string(),
                AudioStream::new("s-1", "mitos-music", StreamKind::Playback, "speakers"),
            );
        }
        Self {
            backend_name: backend.name(),
            backend,
            state: Mutex::new(AudioState {
                devices: HashMap::new(),
                streams,
                default_output: None,
                default_input: None,
                mic_muted: false,
                mic_gain: 0,
                profile: "stereo".into(),
            }),
            events,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    /// Re-scan hardware and reconcile state.
    ///
    /// Emits `DeviceAdded` / `DeviceRemoved` / `DeviceChanged` and fixes up
    /// defaults if a default device vanished. Called at startup, on `Rescan`,
    /// and by the periodic hotplug poller.
    pub async fn refresh(&self) -> Result<(), AudioError> {
        let backend = self.backend.clone();
        let scanned = tokio::task::spawn_blocking(move || backend.scan())
            .await
            .map_err(|e| AudioError::Backend(format!("backend scan task failed: {e}")))??;

        let mut pending: Vec<Event> = Vec::new();
        {
            let mut s = self.state.lock().await;

            // Devices that disappeared.
            let new_ids: HashSet<&str> = scanned.iter().map(|d| d.id.as_str()).collect();
            let removed: Vec<String> = s
                .devices
                .keys()
                .filter(|id| !new_ids.contains(id.as_str()))
                .cloned()
                .collect();
            for id in removed {
                s.devices.remove(&id);
                pending.push(Event::DeviceRemoved { id });
            }

            // Devices that appeared or changed.
            for device in scanned {
                match s.devices.get(&device.id) {
                    None => {
                        pending.push(Event::DeviceAdded {
                            id: device.id.clone(),
                            name: device.name.clone(),
                        });
                        s.devices.insert(device.id.clone(), device);
                    }
                    Some(old) => {
                        // `state` is manager-owned (Active/Available), so it is
                        // excluded from hardware comparison.
                        if !hardware_equal(old, &device) {
                            pending.push(Event::DeviceChanged { id: device.id.clone() });
                            s.devices.insert(device.id.clone(), device);
                        }
                    }
                }
            }

            // Keep defaults valid (and pick initial ones).
            let output = s
                .default_output
                .clone()
                .filter(|id| {
                    s.devices.get(id).map(|d| d.direction != Direction::Input).unwrap_or(false)
                })
                .or_else(|| pick_default_output(&s.devices));
            if output != s.default_output {
                s.default_output = output.clone();
                if let Some(id) = &output {
                    pending.push(Event::DefaultOutputChanged { id: id.clone() });
                }
            }

            let input = s
                .default_input
                .clone()
                .filter(|id| {
                    s.devices.get(id).map(|d| d.direction != Direction::Output).unwrap_or(false)
                })
                .or_else(|| pick_default_input(&s.devices));
            if input != s.default_input {
                s.default_input = input.clone();
                if let Some(id) = &input {
                    pending.push(Event::DefaultInputChanged { id: id.clone() });
                }
            }

            // Cosmetic device states: defaults are "active".
            for d in s.devices.values_mut() {
                d.state = match (&s.default_output, &s.default_input) {
                    (Some(o), _) if d.id == *o => DeviceState::Active,
                    (_, Some(i)) if d.id == *i => DeviceState::Active,
                    _ => DeviceState::Available,
                };
            }
        }
        for event in pending {
            self.emit(event);
        }
        Ok(())
    }

    /// Push a volume change to hardware (off the async executor).
    /// Hardware failures are logged and soft-failed: manager state stays
    /// consistent even on devices without volume controls.
    async fn push_volume(&self, device: &Device, volume: u32) -> bool {
        let backend = self.backend.clone();
        let device = device.clone();
        match tokio::task::spawn_blocking(move || backend.set_volume(&device, volume)).await {
            Ok(Ok(applied)) => applied,
            Ok(Err(e)) => {
                tracing::warn!(backend = self.backend_name, error = %e, "hardware volume write failed");
                false
            }
            Err(e) => {
                tracing::warn!(error = %e, "backend task panicked");
                false
            }
        }
    }

    async fn push_mute(&self, device: &Device, mute: bool) -> bool {
        let backend = self.backend.clone();
        let device = device.clone();
        match tokio::task::spawn_blocking(move || backend.set_mute(&device, mute)).await {
            Ok(Ok(applied)) => applied,
            Ok(Err(e)) => {
                tracing::warn!(backend = self.backend_name, error = %e, "hardware mute write failed");
                false
            }
            Err(e) => {
                tracing::warn!(error = %e, "backend task panicked");
                false
            }
        }
    }

    /// Snapshot of the default output device (master volume target).
    async fn master_output(&self) -> Result<(String, Device), AudioError> {
        let s = self.state.lock().await;
        let id = s.default_output.clone().ok_or(AudioError::NoOutput)?;
        let device = s.devices.get(&id).cloned().ok_or(AudioError::DeviceNotFound(id))?;
        Ok((id, device))
    }

    pub async fn handle(&self, command: Command) -> Result<Value, AudioError> {
        match command {
            Command::Ping => Ok(json!({ "pong": true })),

            Command::GetState => {
                let s = self.state.lock().await;
                Ok(snapshot(&s, self.backend_name))
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
                    let direction = s
                        .devices
                        .get(&id)
                        .map(|d| d.direction)
                        .ok_or_else(|| AudioError::DeviceNotFound(id.clone()))?;
                    if direction == Direction::Input {
                        return Err(AudioError::NotAnOutput(id));
                    }
                    let previous = s.default_output.replace(id.clone());
                    if let Some(old) = previous {
                        if let Some(d) = s.devices.get_mut(&old) {
                            d.state = DeviceState::Available;
                        }
                    }
                    if let Some(d) = s.devices.get_mut(&id) {
                        d.state = DeviceState::Active;
                    }
                }
                self.emit(Event::DefaultOutputChanged { id: id.clone() });
                Ok(json!({ "default_output": id }))
            }

            Command::SetDefaultInput { id } => {
                {
                    let mut s = self.state.lock().await;
                    let direction = s
                        .devices
                        .get(&id)
                        .map(|d| d.direction)
                        .ok_or_else(|| AudioError::DeviceNotFound(id.clone()))?;
                    if direction == Direction::Output {
                        return Err(AudioError::NotAnInput(id));
                    }
                    let previous = s.default_input.replace(id.clone());
                    if let Some(old) = previous {
                        if let Some(d) = s.devices.get_mut(&old) {
                            d.state = DeviceState::Available;
                        }
                    }
                    if let Some(d) = s.devices.get_mut(&id) {
                        d.state = DeviceState::Active;
                    }
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
                    // Master volume = default output device's volume.
                    None => {
                        let id = s.default_output.as_ref().ok_or(AudioError::NoOutput)?;
                        let d = &s.devices[id];
                        Ok(json!({ "device": d.id, "volume": d.volume, "muted": d.muted }))
                    }
                }
            }

            Command::SetVolume { device, volume } => {
                if volume > self.config.max_volume {
                    return Err(AudioError::InvalidVolume(volume));
                }
                let (target, snapshot_dev) = {
                    let s = self.state.lock().await;
                    let id = match device {
                        Some(ref id) => id.clone(),
                        None => s.default_output.clone().ok_or(AudioError::NoOutput)?,
                    };
                    let d = s.devices.get(&id)
                        .ok_or_else(|| AudioError::DeviceNotFound(id.clone()))?;
                    (id, d.clone())
                };
                let applied = self.push_volume(&snapshot_dev, volume).await;
                {
                    let mut s = self.state.lock().await;
                    if let Some(d) = s.devices.get_mut(&target) {
                        d.volume = volume;
                    }
                }
                self.emit(Event::VolumeChanged { device: Some(target.clone()), volume });
                Ok(json!({ "device": target, "volume": volume, "hw_applied": applied }))
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
                let device = s.default_input.as_ref().and_then(|id| s.devices.get(id)).cloned();
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
                let (volume, muted) = s
                    .default_output
                    .as_ref()
                    .and_then(|id| s.devices.get(id))
                    .map(|d| (d.volume, d.muted))
                    .unwrap_or((0, false));
                Ok(json!({
                    "master_volume": volume,
                    "muted": muted,
                    // Real metering arrives with the PCM monitoring work (roadmap).
                    "output_level": 0.0,
                    "input_level": 0.0,
                    "peak": 0.0,
                    "clipping": false,
                }))
            }

            Command::Rescan => {
                self.refresh().await?;
                let s = self.state.lock().await;
                Ok(json!({
                    "devices": sorted_devices(&s),
                    "default_output": s.default_output,
                    "default_input": s.default_input,
                }))
            }

            // Normally intercepted by the connection handler (needs the socket).
            Command::SubscribeEvents => Ok(json!({ "subscribed": true })),
        }
    }

    async fn set_master_muted(&self, muted: bool) -> Result<Value, AudioError> {
        let (target, snapshot_dev) = {
            let s = self.state.lock().await;
            let id = s.default_output.clone().ok_or(AudioError::NoOutput)?;
            let device = s.devices.get(&id).cloned()
                .ok_or_else(|| AudioError::DeviceNotFound(id.clone()))?;
            (id, device)
        };
        let applied = self.push_mute(&snapshot_dev, muted).await;
        {
            let mut s = self.state.lock().await;
            if let Some(d) = s.devices.get_mut(&target) {
                d.muted = muted;
            }
        }
        self.emit(Event::MuteChanged { muted });
        Ok(json!({ "device": target, "muted": muted, "hw_applied": applied }))
    }
}

// ─── helpers ────────────────────────────────────────────────────────────

fn sorted_devices(s: &AudioState) -> Vec<&Device> {
    let mut devices: Vec<&Device> = s.devices.values().collect();
    devices.sort_by(|a, b| a.id.cmp(&b.id));
    devices
}

fn snapshot(s: &AudioState, backend: &'static str) -> Value {
    let devices = sorted_devices(s);
    let mut streams: Vec<&AudioStream> = s.streams.values().collect();
    streams.sort_by(|a, b| a.id.cmp(&b.id));
    let (master_volume, master_muted) = s
        .default_output
        .as_ref()
        .and_then(|id| s.devices.get(id))
        .map(|d| (d.volume, d.muted))
        .unwrap_or((0, false));
    json!({
        "service": "mitos-audio",
        "version": env!("CARGO_PKG_VERSION"),
        "backend": backend,
        "devices": devices,
        "streams": streams,
        "default_output": s.default_output,
        "default_input": s.default_input,
        "master_volume": master_volume,
        "master_muted": master_muted,
        "microphone": { "muted": s.mic_muted, "gain_db": s.mic_gain },
        "profile": s.profile,
    })
}

fn pick_default_output(devices: &HashMap<String, Device>) -> Option<String> {
    fn rank(d: &Device) -> u8 {
        match d.kind {
            DeviceKind::Speakers | DeviceKind::Headphones => 0,
            DeviceKind::Headset => 1,
            DeviceKind::Usb => 2,
            DeviceKind::Hdmi | DeviceKind::DisplayPort => 4,
            _ => 3,
        }
    }
    devices
        .values()
        .filter(|d| d.direction != Direction::Input)
        .min_by_key(|d| (rank(d), d.id.clone()))
        .map(|d| d.id.clone())
}

fn pick_default_input(devices: &HashMap<String, Device>) -> Option<String> {
    devices
        .values()
        .filter(|d| d.direction != Direction::Output)
        .min_by_key(|d| (if d.kind == DeviceKind::Microphone { 0 } else { 1 }, d.id.clone()))
        .map(|d| d.id.clone())
}

/// Hardware-level equality — excludes `state` (manager-owned).
fn hardware_equal(a: &Device, b: &Device) -> bool {
    a.id == b.id
        && a.name == b.name
        && a.description == b.description
        && a.kind == b.kind
        && a.direction == b.direction
        && a.bus == b.bus
        && a.profiles == b.profiles
        && a.active_profile == b.active_profile
        && a.channels == b.channels
        && a.sample_rates == b.sample_rates
        && a.volume == b.volume
        && a.muted == b.muted
        && a.alsa == b.alsa
}