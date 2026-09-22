use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::{broadcast, Mutex};

use crate::backend::AudioBackend;
use crate::config::AudioConfig;
use crate::devices::device::{Device, DeviceKind, DeviceState, Direction};
use crate::errors::AudioError;
use crate::ipc::messages::{Command, Event};
use crate::monitoring::LevelFrame;
use crate::persistence::{DeviceSettings, Settings, SettingsStore};
use crate::routing::{ResolvedAction, RoutingEngine, Trigger, TriggerContext};
use crate::streams::stream::{AudioStream, StreamKind};

const KNOWN_PROFILES: &[&str] = &[
    "stereo", "headphones", "headset", "hdmi-stereo", "surround-5.1",
    "surround-7.1", "bluetooth-music", "bluetooth-headset", "usb-dac",
];

/// Central audio state, command dispatch, routing execution and persistence.
///
/// The backend owns *hardware truth*; the manager owns policy (defaults,
/// profiles, mic, routing) and is the only component that touches the
/// backend or the persistent store.
pub struct AudioManager {
    config: AudioConfig,
    backend: Arc<dyn AudioBackend>,
    backend_name: &'static str,
    state: Mutex<AudioState>,
    events: broadcast::Sender<Event>,
    routing: RoutingEngine,
    store: Option<SettingsStore>,
    last_levels: Mutex<LevelFrame>,
    next_stream_id: AtomicU64,
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
        store: Option<SettingsStore>,
    ) -> Self {
        let routing = RoutingEngine::load(&config.routing_path);

        let mut streams = HashMap::new();
        if backend.name() == "demo" {
            streams.insert(
                "demo-1".to_string(),
                AudioStream::new("demo-1", "mitos-music", StreamKind::Playback, "speakers"),
            );
        }

        Self {
            backend_name: backend.name(),
            backend,
            routing,
            store,
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
            last_levels: Mutex::new(LevelFrame::default()),
            next_stream_id: AtomicU64::new(1),
            config,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    pub fn routing(&self) -> &RoutingEngine {
        &self.routing
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    /// Last measured level frame (updated by the daemon's meter task).
    pub async fn update_levels(&self, frame: LevelFrame) {
        *self.last_levels.lock().await = frame;
    }

    // ════════════════════════════════════════════════════════════════════
    // Refresh: scan hardware, diff, run routing rules, persist
    // ════════════════════════════════════════════════════════════════════

    pub async fn refresh(&self) -> Result<(), AudioError> {
        let backend = self.backend.clone();
        let scanned = tokio::task::spawn_blocking(move || backend.scan())
            .await
            .map_err(|e| AudioError::Backend(format!("backend scan task failed: {e}")))??;

        let mut events: Vec<Event> = Vec::new();
        let mut added: Vec<Device> = Vec::new();
        let mut removed: Vec<Device> = Vec::new();

        {
            let mut s = self.state.lock().await;

            // Devices that disappeared (snapshot kept for routing rules).
            let new_ids: HashSet<&str> = scanned.iter().map(|d| d.id.as_str()).collect();
            let gone: Vec<String> = s
                .devices
                .keys()
                .filter(|id| !new_ids.contains(id.as_str()))
                .cloned()
                .collect();
            for id in gone {
                if let Some(device) = s.devices.remove(&id) {
                    removed.push(device);
                }
                events.push(Event::DeviceRemoved { id });
            }

            // Devices that appeared or changed.
            for device in scanned {
                match s.devices.get(&device.id) {
                    None => {
                        events.push(Event::DeviceAdded {
                            id: device.id.clone(),
                            name: device.name.clone(),
                        });
                        added.push(device.clone());
                        s.devices.insert(device.id.clone(), device);
                    }
                    Some(old) => {
                        // `state` is manager-owned — excluded from comparison.
                        if !hardware_equal(old, &device) {
                            events.push(Event::DeviceChanged { id: device.id.clone() });
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
                    events.push(Event::DefaultOutputChanged { id: id.clone() });
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
                    events.push(Event::DefaultInputChanged { id: id.clone() });
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

        // Routing rules for device triggers.
        if !added.is_empty() || !removed.is_empty() {
            let profile = self.state.lock().await.profile.clone();
            for device in added.iter().chain(removed.iter()) {
                let trigger = if added.iter().any(|d| d.id == device.id) {
                    Trigger::DeviceAdded
                } else {
                    Trigger::DeviceRemoved
                };
                let ctx = TriggerContext { trigger, device: Some(device), stream: None, profile: &profile };
                events.extend(self.apply_routing(self.routing.evaluate(&ctx)).await);
            }
        }

        for event in events {
            self.emit(event);
        }
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // Restore persisted settings (called once after the initial scan)
    // ════════════════════════════════════════════════════════════════════

    pub async fn restore(&self, settings: Settings) {
        let mut volume_pushes: Vec<(Device, u32)> = Vec::new();
        let mut mute_pushes: Vec<(Device, bool)> = Vec::new();
        let mut events: Vec<Event> = Vec::new();

        {
            let mut s = self.state.lock().await;

            if let Some(id) = &settings.default_output {
                let valid = s
                    .devices
                    .get(id)
                    .map(|d| d.direction != Direction::Input)
                    .unwrap_or(false);
                if valid && s.default_output.as_deref() != Some(id.as_str()) {
                    if let Some(old) = s.default_output.take() {
                        if let Some(d) = s.devices.get_mut(&old) {
                            d.state = DeviceState::Available;
                        }
                    }
                    s.default_output = Some(id.clone());
                    if let Some(d) = s.devices.get_mut(id) {
                        d.state = DeviceState::Active;
                    }
                    events.push(Event::DefaultOutputChanged { id: id.clone() });
                }
            }
            if let Some(id) = &settings.default_input {
                let valid = s
                    .devices
                    .get(id)
                    .map(|d| d.direction != Direction::Output)
                    .unwrap_or(false);
                if valid && s.default_input.as_deref() != Some(id.as_str()) {
                    if let Some(old) = s.default_input.take() {
                        if let Some(d) = s.devices.get_mut(&old) {
                            d.state = DeviceState::Available;
                        }
                    }
                    s.default_input = Some(id.clone());
                    if let Some(d) = s.devices.get_mut(id) {
                        d.state = DeviceState::Active;
                    }
                    events.push(Event::DefaultInputChanged { id: id.clone() });
                }
            }
            if let Some(profile) = &settings.profile {
                if KNOWN_PROFILES.contains(&profile.as_str()) && s.profile != *profile {
                    s.profile = profile.clone();
                    events.push(Event::ProfileChanged { profile: profile.clone() });
                }
            }
            s.mic_muted = settings.mic_muted;
            s.mic_gain = settings.mic_gain;

            for (id, ds) in &settings.devices {
                let Some(d) = s.devices.get_mut(id) else { continue };
                if let Some(volume) = ds.volume {
                    if volume <= self.config.max_volume && d.volume != volume {
                        d.volume = volume;
                        volume_pushes.push((d.clone(), volume));
                    }
                }
                if let Some(muted) = ds.muted {
                    if d.muted != muted {
                        d.muted = muted;
                        mute_pushes.push((d.clone(), muted));
                    }
                }
            }
        }

        self.routing.restore_memory(settings.routing);

        for (device, volume) in &volume_pushes {
            self.push_volume(device, *volume).await;
        }
        for (device, muted) in &mute_pushes {
            self.push_mute(device, *muted).await;
        }
        for event in events {
            self.emit(event);
        }
        if !events.is_empty() || !volume_pushes.is_empty() || !mute_pushes.is_empty() {
            self.request_save().await;
        }
        tracing::info!(events = events.len(), "persisted settings restored");
    }

    // ════════════════════════════════════════════════════════════════════
    // Routing execution (shared by refresh / CreateStream / SetProfile)
    // ════════════════════════════════════════════════════════════════════

    async fn apply_routing(&self, actions: Vec<ResolvedAction>) -> Vec<Event> {
        let mut events = Vec::new();
        let mut memory_changed = false;

        for action in actions {
            match action {
                ResolvedAction::SetDefaultOutput { id, remember } => {
                    if let Some(previous) = self.switch_default_output(&id).await {
                        if remember && previous != id {
                            self.routing.note_output_switch(Some(previous));
                            memory_changed = true;
                        }
                        events.push(Event::DefaultOutputChanged { id });
                    }
                }
                ResolvedAction::SetDefaultInput { id } => {
                    if let Some(previous) = self.switch_default_input(&id).await {
                        let _ = previous;
                        events.push(Event::DefaultInputChanged { id });
                    }
                }
                ResolvedAction::SetProfile { profile } => {
                    let changed = {
                        let mut s = self.state.lock().await;
                        if KNOWN_PROFILES.contains(&profile.as_str()) && s.profile != profile {
                            s.profile = profile.clone();
                            true
                        } else {
                            false
                        }
                    };
                    if changed {
                        events.push(Event::ProfileChanged { profile });
                    }
                }
                ResolvedAction::MoveStream { stream_id, to } => {
                    let changed = {
                        let mut s = self.state.lock().await;
                        let device_ok = s
                            .devices
                            .get(&to)
                            .map(|d| d.direction != Direction::Input)
                            .unwrap_or(false);
                        match (device_ok, s.streams.get_mut(&stream_id)) {
                            (true, Some(stream)) if stream.device != to => {
                                stream.device = to.clone();
                                true
                            }
                            _ => false,
                        }
                    };
                    if changed {
                        events.push(Event::StreamChanged { id: stream_id });
                    }
                }
                ResolvedAction::RestoreOutput => {
                    if let Some(id) = self.routing.take_previous_output() {
                        memory_changed = true;
                        if self.switch_default_output(&id).await.is_some() {
                            events.push(Event::DefaultOutputChanged { id });
                        }
                    }
                }
                ResolvedAction::RestoreInput => {
                    if let Some(id) = self.routing.take_previous_input() {
                        memory_changed = true;
                        if self.switch_default_input(&id).await.is_some() {
                            events.push(Event::DefaultInputChanged { id });
                        }
                    }
                }
            }
        }

        if !events.is_empty() || memory_changed {
            self.request_save().await;
        }
        events
    }

    /// Set the default output. Returns the previous default when the
    /// switch actually happened (device valid + different from current).
    async fn switch_default_output(&self, id: &str) -> Option<String> {
        let mut s = self.state.lock().await;
        let valid = s
            .devices
            .get(id)
            .map(|d| d.direction != Direction::Input)
            .unwrap_or(false);
        if !valid || s.default_output.as_deref() == Some(id) {
            return None;
        }
        let previous = s.default_output.replace(id.to_string());
        if let Some(old) = &previous {
            if let Some(d) = s.devices.get_mut(old) {
                d.state = DeviceState::Available;
            }
        }
        if let Some(d) = s.devices.get_mut(id) {
            d.state = DeviceState::Active;
        }
        previous
    }

    async fn switch_default_input(&self, id: &str) -> Option<String> {
        let mut s = self.state.lock().await;
        let valid = s
            .devices
            .get(id)
            .map(|d| d.direction != Direction::Output)
            .unwrap_or(false);
        if !valid || s.default_input.as_deref() == Some(id) {
            return None;
        }
        let previous = s.default_input.replace(id.to_string());
        if let Some(old) = &previous {
            if let Some(d) = s.devices.get_mut(old) {
                d.state = DeviceState::Available;
            }
        }
        if let Some(d) = s.devices.get_mut(id) {
            d.state = DeviceState::Active;
        }
        previous
    }

    // ════════════════════════════════════════════════════════════════════
    // Hardware helpers + persistence
    // ════════════════════════════════════════════════════════════════════

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

    /// Snapshot current settings into the debounced persistence writer.
    async fn request_save(&self) {
        let Some(store) = &self.store else { return };
        let settings = {
            let s = self.state.lock().await;
            Settings {
                default_output: s.default_output.clone(),
                default_input: s.default_input.clone(),
                profile: Some(s.profile.clone()),
                mic_muted: s.mic_muted,
                mic_gain: s.mic_gain,
                devices: s
                    .devices
                    .iter()
                    .map(|(id, d)| {
                        (
                            id.clone(),
                            DeviceSettings {
                                volume: Some(d.volume),
                                muted: Some(d.muted),
                            },
                        )
                    })
                    .collect(),
                routing: self.routing.memory(),
            }
        };
        store.request_save(settings);
    }

    // ════════════════════════════════════════════════════════════════════
    // Command dispatch
    // ════════════════════════════════════════════════════════════════════

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
                    let s = self.state.lock().await;
                    let d = s.devices.get(&id)
                        .ok_or_else(|| AudioError::DeviceNotFound(id.clone()))?;
                    if d.direction == Direction::Input {
                        return Err(AudioError::NotAnOutput(id));
                    }
                }
                if self.switch_default_output(&id).await.is_some() {
                    self.emit(Event::DefaultOutputChanged { id: id.clone() });
                    self.request_save().await;
                }
                Ok(json!({ "default_output": id }))
            }

            Command::SetDefaultInput { id } => {
                {
                    let s = self.state.lock().await;
                    let d = s.devices.get(&id)
                        .ok_or_else(|| AudioError::DeviceNotFound(id.clone()))?;
                    if d.direction == Direction::Output {
                        return Err(AudioError::NotAnInput(id));
                    }
                }
                if self.switch_default_input(&id).await.is_some() {
                    self.emit(Event::DefaultInputChanged { id: id.clone() });
                    self.request_save().await;
                }
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
                self.request_save().await;
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

            Command::CreateStream { application, device, kind } => {
                let kind = parse_stream_kind(&kind);
                let (stream, device_snapshot) = {
                    let mut s = self.state.lock().await;
                    let device_id = match device {
                        Some(ref id) => {
                            if !s.devices.contains_key(id) {
                                return Err(AudioError::DeviceNotFound(id.clone()));
                            }
                            id.clone()
                        }
                        None => match kind {
                            StreamKind::Playback | StreamKind::Monitoring => {
                                s.default_output.clone().ok_or(AudioError::NoOutput)?
                            }
                            _ => s.default_input.clone().ok_or(AudioError::NoInput)?,
                        },
                    };
                    let id =
                        format!("s-{}", self.next_stream_id.fetch_add(1, Ordering::Relaxed));
                    let stream = AudioStream::new(&id, &application, kind, &device_id);
                    s.streams.insert(id, stream.clone());
                    (stream, s.devices.get(&device_id).cloned())
                };

                // Routing rules for stream-added.
                let routing_events = {
                    let profile = self.state.lock().await.profile.clone();
                    let ctx = TriggerContext {
                        trigger: Trigger::StreamAdded,
                        device: device_snapshot.as_ref(),
                        stream: Some(&stream),
                        profile: &profile,
                    };
                    self.apply_routing(self.routing.evaluate(&ctx)).await
                };

                self.emit(Event::StreamAdded {
                    id: stream.id.clone(),
                    application: stream.application.clone(),
                });
                for event in routing_events {
                    self.emit(event);
                }
                Ok(json!({ "stream": stream }))
            }

            Command::DestroyStream { stream_id } => {
                {
                    let mut s = self.state.lock().await;
                    s.streams.remove(&stream_id)
                        .ok_or_else(|| AudioError::StreamNotFound(stream_id.clone()))?;
                }
                self.emit(Event::StreamRemoved { id: stream_id.clone() });
                Ok(json!({ "removed": stream_id }))
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
                // Routing rules keyed on the new profile.
                let routing_events = {
                    let ctx = TriggerContext {
                        trigger: Trigger::ProfileChanged,
                        device: None,
                        stream: None,
                        profile: &profile,
                    };
                    self.apply_routing(self.routing.evaluate(&ctx)).await
                };
                for event in routing_events {
                    self.emit(event);
                }
                self.request_save().await;
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
                self.request_save().await;
                Ok(json!({ "gain_db": clamped }))
            }

            Command::MuteMicrophone { mute } => {
                let gain = {
                    let mut s = self.state.lock().await;
                    s.mic_muted = mute;
                    s.mic_gain
                };
                self.emit(Event::MicrophoneChanged { muted: mute, gain });
                self.request_save().await;
                Ok(json!({ "muted": mute }))
            }

            Command::GetLevels => {
                let frame = *self.last_levels.lock().await;
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
                    "output_level": frame.output_level,
                    "input_level": frame.input_level,
                    "peak": frame.peak,
                    "clipping": frame.clipping,
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

            Command::ReloadRouting => {
                let count = self.routing.reload(&self.config.routing_path)?;
                tracing::info!(rules = count, "routing rules reloaded");
                Ok(json!({ "rules": count }))
            }

            // Normally intercepted by the connection handler (needs the socket).
            Command::SubscribeEvents | Command::SubscribeLevels => {
                Ok(json!({ "subscribed": true }))
            }
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
        self.request_save().await;
        Ok(json!({ "device": target, "muted": muted, "hw_applied": applied }))
    }
}

// ─── helpers ────────────────────────────────────────────────────────────

fn parse_stream_kind(kind: &Option<String>) -> StreamKind {
    match kind.as_deref().map(str::to_lowercase).as_deref() {
        Some("recording") => StreamKind::Recording,
        Some("capture") => StreamKind::Capture,
        Some("monitoring") => StreamKind::Monitoring,
        _ => StreamKind::Playback,
    }
}

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