//! Output sinks: one output device + mixing thread per target device.
//! Streams target a device via their `device` field; sinks are created
//! lazily for referenced devices and closed when idle.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::backend::{AudioBackend, OutputDevice};
use crate::devices::device::Device;
use crate::effects::EffectsChain;
use crate::errors::AudioError;
use crate::groups::{compute_delays_ms, SpeakerGroup};

use super::{MIX_CHANNELS, MIX_RATE, OutputLevels, PlaybackReference, StreamHub};

/// Frames per mixer period (12.5 ms @ 48 kHz stereo).
const PERIOD_FRAMES: usize = 600;
/// Close a sink after this long with no streams targeting it.
const IDLE_CLOSE: Duration = Duration::from_secs(5);
/// Minimum delay between open attempts for the same device (busy, etc.).
const OPEN_RETRY: Duration = Duration::from_secs(2);

pub struct SinkManager {
    hub: Arc<StreamHub>,
    backend: Arc<dyn AudioBackend>,
    levels: Arc<OutputLevels>,
    sinks: Mutex<HashMap<String, SinkHandle>>,
    last_attempt: Mutex<HashMap<String, Instant>>,
    /// One effects chain per device, created on first reference (from
    /// either a `manager::state` effects command or a sink opening,
    /// whichever happens first) and kept for the daemon's lifetime so
    /// settings survive the sink closing when idle and reopening later.
    effects: Mutex<HashMap<String, Arc<Mutex<EffectsChain>>>>,
    /// The last-mixed period, shared with `engine::source` for echo
    /// cancellation reference — see `PlaybackReference`.
    playback_reference: Arc<PlaybackReference>,
}

struct SinkHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    last_used: Instant,
    /// For a group sink: the member device ids it was opened with, so
    /// `tick()` can notice when the connected set changes (a speaker was
    /// unplugged, or one that was missing came back) and reopen the group
    /// with the current membership — playback continues on whichever
    /// members are present. `None` for a normal single-device sink.
    group_members: Option<Vec<String>>,
}

impl SinkManager {
    pub fn new(
        hub: Arc<StreamHub>,
        backend: Arc<dyn AudioBackend>,
        levels: Arc<OutputLevels>,
        playback_reference: Arc<PlaybackReference>,
    ) -> Self {
        Self {
            hub,
            backend,
            levels,
            sinks: Mutex::new(HashMap::new()),
            last_attempt: Mutex::new(HashMap::new()),
            effects: Mutex::new(HashMap::new()),
            playback_reference,
        }
    }

    /// The shared effects chain for `device_id`, created (disabled, flat)
    /// on first reference. `manager::state`'s effects commands configure
    /// it; `mixer_loop` applies it once per period.
    pub fn effects_for(&self, device_id: &str) -> Arc<Mutex<EffectsChain>> {
        self.lock_effects()
            .entry(device_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(EffectsChain::new(MIX_RATE as f32, MIX_CHANNELS))))
            .clone()
    }

    fn lock_effects(&self) -> MutexGuard<'_, HashMap<String, Arc<Mutex<EffectsChain>>>> {
        self.effects.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Lifecycle pass (~every 500 ms): ensure sinks exist for every
    /// referenced device *or group*; close idle sinks and sinks whose
    /// target vanished. `devices` is the current device registry;
    /// `groups` the current speaker-group registry (a referenced id is
    /// looked up in `devices` first, then `groups` — see `crate::groups`).
    pub async fn tick(&self, devices: &HashMap<String, Device>, groups: &HashMap<String, SpeakerGroup>) {
        let referenced = self.hub.referenced_devices();
        let mut to_close: HashSet<String> = HashSet::new();

        {
            let mut sinks = self.lock_sinks();
            for id in &referenced {
                if let Some(handle) = sinks.get_mut(id) {
                    handle.last_used = Instant::now();
                }
            }
            for (id, handle) in sinks.iter() {
                let target_exists = devices.contains_key(id) || groups.contains_key(id);
                if !target_exists {
                    to_close.insert(id.clone());
                } else if !referenced.contains(id) && handle.last_used.elapsed() >= IDLE_CLOSE {
                    to_close.insert(id.clone());
                } else if let (Some(opened), Some(group)) = (&handle.group_members, groups.get(id)) {
                    // A speaker was unplugged, or one that was missing came
                    // back: reopen (closed here, reopened by the loop below
                    // on the next tick) so playback continues on whichever
                    // members are actually connected right now.
                    let mut connected_now: Vec<&str> = group
                        .members
                        .iter()
                        .filter(|m| devices.contains_key(&m.device_id))
                        .map(|m| m.device_id.as_str())
                        .collect();
                    let mut opened_sorted: Vec<&str> = opened.iter().map(String::as_str).collect();
                    connected_now.sort_unstable();
                    opened_sorted.sort_unstable();
                    if connected_now != opened_sorted {
                        to_close.insert(id.clone());
                    }
                }
            }
        }

        for id in &referenced {
            if self.lock_sinks().contains_key(id) {
                continue;
            }

            {
                let mut attempts = self.lock_attempts();
                let last = attempts.get(id).map(|t| t.elapsed()).unwrap_or(Duration::MAX);
                if last < OPEN_RETRY {
                    continue;
                }
                attempts.insert(id.clone(), Instant::now());
            }

            if let Some(group) = groups.get(id) {
                self.open_group_sink(id, group, devices).await;
                continue;
            }

            let Some(device) = devices.get(id) else { continue };
            let device = device.clone();
            let backend = self.backend.clone();
            match tokio::task::spawn_blocking(move || backend.open_output(&device)).await {
                Ok(Ok(out)) => {
                    let stop = Arc::new(AtomicBool::new(false));
                    let stop_thread = Arc::clone(&stop);
                    let hub = self.hub.clone();
                    let levels = self.levels.clone();
                    let effects = self.effects_for(id);
                    let playback_reference = self.playback_reference.clone();
                    let thread_name = id.clone();
                    let join = std::thread::Builder::new()
                        .name(format!("mitos-sink-{thread_name}"))
                        .spawn(move || {
                            mixer_loop(thread_name, hub, out, levels, effects, playback_reference, stop_thread)
                        })
                        .ok();
                    self.lock_sinks().insert(
                        id.clone(),
                        SinkHandle { stop, join, last_used: Instant::now(), group_members: None },
                    );
                    tracing::info!(device = %id, "output sink started");
                }
                Ok(Err(e)) => {
                    tracing::debug!(device = %id, error = %e, "output open failed — retrying later")
                }
                Err(e) => {
                    tracing::debug!(device = %id, error = %e, "output open task failed")
                }
            }
        }

        for id in to_close {
            self.close_sink(&id).await;
        }
    }

    /// Open every connected member of `group` and spawn its
    /// [`group_mixer_loop`]. Members not currently connected are skipped
    /// (not fatal) — a group plays on whichever of its members are
    /// present; a group with none connected right now simply doesn't
    /// start yet and is retried next tick, same as a single device would
    /// be while temporarily unavailable.
    async fn open_group_sink(&self, id: &str, group: &SpeakerGroup, devices: &HashMap<String, Device>) {
        let delays: HashMap<String, u32> = compute_delays_ms(&group.members).into_iter().collect();
        let connected: Vec<(Device, u32)> = group
            .members
            .iter()
            .filter_map(|m| {
                let device = devices.get(&m.device_id)?;
                let delay_ms = *delays.get(&m.device_id).unwrap_or(&0);
                Some((device.clone(), delay_ms))
            })
            .collect();

        if connected.is_empty() {
            tracing::debug!(group = %id, "no connected member devices yet — will retry");
            return;
        }

        let backend = self.backend.clone();
        let to_open = connected.clone();
        let opened = tokio::task::spawn_blocking(move || {
            to_open
                .into_iter()
                .map(|(device, delay_ms)| {
                    let id = device.id.clone();
                    backend.open_output(&device).map(|out| (id, out, delay_ms))
                })
                .collect::<Result<Vec<_>, AudioError>>()
        })
        .await;

        match opened {
            Ok(Ok(outs)) => {
                let member_count = outs.len();
                let opened_ids: Vec<String> = outs.iter().map(|(member_id, _, _)| member_id.clone()).collect();
                let members: Vec<(String, Box<dyn OutputDevice>, DelayLine)> = outs
                    .into_iter()
                    .map(|(member_id, out, delay_ms)| {
                        let delay_frames = (delay_ms as usize * MIX_RATE as usize) / 1000;
                        (member_id, out, DelayLine::new(delay_frames, MIX_CHANNELS))
                    })
                    .collect();

                let stop = Arc::new(AtomicBool::new(false));
                let stop_thread = Arc::clone(&stop);
                let hub = self.hub.clone();
                let levels = self.levels.clone();
                let effects = self.effects_for(id);
                let playback_reference = self.playback_reference.clone();
                let thread_name = id.to_string();
                let join = std::thread::Builder::new()
                    .name(format!("mitos-group-{thread_name}"))
                    .spawn(move || {
                        group_mixer_loop(thread_name, hub, members, levels, effects, playback_reference, stop_thread)
                    })
                    .ok();
                self.lock_sinks().insert(
                    id.to_string(),
                    SinkHandle { stop, join, last_used: Instant::now(), group_members: Some(opened_ids) },
                );
                tracing::info!(group = %id, members = member_count, "group sink started");
            }
            Ok(Err(e)) => tracing::debug!(group = %id, error = %e, "group open failed — retrying later"),
            Err(e) => tracing::debug!(group = %id, error = %e, "group open task failed"),
        }
    }

    /// Stop and join the sink for `device_id` (device removed or idle).
    pub async fn close_sink(&self, device_id: &str) {
        let handle = self.lock_sinks().remove(device_id);
        if let Some(mut handle) = handle {
            handle.stop.store(true, Ordering::Relaxed);
            if let Some(join) = handle.join.take() {
                let _ = tokio::task::spawn_blocking(move || {
                    let _ = join.join();
                })
                .await;
            }
            tracing::info!(device = device_id, "output sink closed");
        }
    }

    pub fn sink_count(&self) -> usize {
        self.lock_sinks().len()
    }

    fn lock_sinks(&self) -> MutexGuard<'_, HashMap<String, SinkHandle>> {
        self.sinks.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_attempts(&self) -> MutexGuard<'_, HashMap<String, Instant>> {
        self.last_attempt.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn mixer_loop(
    device_id: String,
    hub: Arc<StreamHub>,
    mut out: Box<dyn OutputDevice>,
    levels: Arc<OutputLevels>,
    effects: Arc<Mutex<EffectsChain>>,
    playback_reference: Arc<PlaybackReference>,
    stop: Arc<AtomicBool>,
) {
    let mut mix = vec![0.0f32; PERIOD_FRAMES * MIX_CHANNELS];
    let mut read = vec![0.0f32; PERIOD_FRAMES * MIX_CHANNELS];

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        mix_period(&device_id, &hub, &mut mix, &mut read, &effects);
        playback_reference.set(&mix);
        levels.update(&mix);

        if let Err(e) = write_all(&mut out, &mix) {
            tracing::warn!(device = %device_id, error = %e, "sink write failed — recovering");
            if out.recover().is_err() {
                tracing::error!(device = %device_id, "sink unrecoverable — exiting mixer thread");
                break;
            }
        }
    }
}

/// Mix every live stream targeting `target_id` into `mix` and apply
/// `effects`. `target_id` is a real device id for a normal sink, or a
/// group id for a group sink — `hub`/`LiveStream` don't distinguish
/// (a stream's `device` field is just a string), which is exactly what
/// lets a group act as a virtual device with no changes needed anywhere
/// upstream of this. Shared by [`mixer_loop`] and [`group_mixer_loop`] so
/// the two can never mix audio differently from each other.
fn mix_period(
    target_id: &str,
    hub: &StreamHub,
    mix: &mut [f32],
    read: &mut [f32],
    effects: &Mutex<EffectsChain>,
) {
    mix.iter_mut().for_each(|s| *s = 0.0);

    for live in hub.snapshot() {
        if live.device() != target_id {
            continue;
        }
        let frames = live.take(read);
        if frames == 0 {
            live.note_starved();
            continue;
        }
        if live.muted() {
            continue; // pulled and discarded — no stale audio on unmute
        }
        // Perceptual (squared) volume curve — see docs/audio-plane.md.
        let scale = (f32::from(live.volume()) / 100.0).powi(2);
        for i in 0..frames * MIX_CHANNELS {
            mix[i] += read[i] * scale;
        }
    }

    for s in mix.iter_mut() {
        *s = s.clamp(-1.0, 1.0);
    }

    // Per-device EQ/compressor/spatial/limiter — see crate::effects.
    // Applied before metering so levels/clipping reflect what actually
    // reaches hardware, not the pre-effects mix. For a group, this is the
    // *group's* effects chain (one chain per virtual/real target id,
    // keyed the same way — see `SinkManager::effects_for`), applied once,
    // identically, before fan-out — not per member.
    effects.lock().unwrap_or_else(|e| e.into_inner()).process(mix);
}

/// A fixed-length FIFO delay: pre-filled with `delay_frames` of silence,
/// then each call pushes `input.len()` new frames in and pops the same
/// count of now-delayed frames out. Used for per-member latency
/// compensation in [`group_mixer_loop`] — see `crate::groups::compute_delays_ms`.
struct DelayLine {
    buffer: std::collections::VecDeque<f32>,
}

impl DelayLine {
    fn new(delay_frames: usize, channels: usize) -> Self {
        let mut buffer = std::collections::VecDeque::with_capacity((delay_frames + PERIOD_FRAMES) * channels);
        buffer.resize(delay_frames * channels, 0.0);
        Self { buffer }
    }

    fn process(&mut self, input: &[f32]) -> Vec<f32> {
        self.buffer.extend(input.iter().copied());
        (0..input.len()).map(|_| self.buffer.pop_front().unwrap_or(0.0)).collect()
    }
}

/// The group-sink mirror of `mixer_loop`: mix exactly once (via the same
/// [`mix_period`] every normal sink uses), then deliver that identical
/// buffer to every member, each through its own [`DelayLine`] so a
/// lower-latency member doesn't play ahead of a higher-latency one. One
/// member's write failure only recovers/drops that member — it doesn't
/// stop audio reaching the others.
fn group_mixer_loop(
    group_id: String,
    hub: Arc<StreamHub>,
    mut members: Vec<(String, Box<dyn OutputDevice>, DelayLine)>,
    levels: Arc<OutputLevels>,
    effects: Arc<Mutex<EffectsChain>>,
    playback_reference: Arc<PlaybackReference>,
    stop: Arc<AtomicBool>,
) {
    let mut mix = vec![0.0f32; PERIOD_FRAMES * MIX_CHANNELS];
    let mut read = vec![0.0f32; PERIOD_FRAMES * MIX_CHANNELS];

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        mix_period(&group_id, &hub, &mut mix, &mut read, &effects);
        playback_reference.set(&mix);
        levels.update(&mix);

        let mut any_ok = false;
        for (member_id, out, delay) in members.iter_mut() {
            let delayed = delay.process(&mix);
            match write_all(out, &delayed) {
                Ok(()) => any_ok = true,
                Err(e) => {
                    tracing::warn!(group = %group_id, device = %member_id.as_str(), error = %e, "group member write failed — recovering");
                    let _ = out.recover();
                }
            }
        }
        if !any_ok {
            // Normally a member's blocking hardware write is what paces this
            // loop; a dead member fails instantly instead. If *every* member
            // just failed, back off briefly rather than spinning until the
            // next `tick()` notices the topology change and reopens the group.
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn write_all(out: &mut Box<dyn OutputDevice>, frames: &[f32]) -> Result<(), AudioError> {
    let total = frames.len() / MIX_CHANNELS;
    let mut written = 0;
    while written < total {
        let n = out.write(&frames[written * MIX_CHANNELS..])?;
        if n == 0 {
            return Err(AudioError::Backend("output wrote 0 frames".into()));
        }
        written += n;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_line_outputs_silence_for_the_configured_delay_then_the_real_signal() {
        let mut line = DelayLine::new(3, 1); // delay by 3 frames, mono
        let out1 = line.process(&[1.0, 2.0]);
        assert_eq!(out1, vec![0.0, 0.0]); // still draining the 3-frame pre-fill
        let out2 = line.process(&[3.0, 4.0]);
        assert_eq!(out2, vec![0.0, 1.0]); // last bit of pre-fill, then frame 1
        let out3 = line.process(&[5.0, 6.0]);
        assert_eq!(out3, vec![2.0, 3.0]); // fully caught up to the 3-frame delay
    }

    #[test]
    fn zero_delay_passes_through_unchanged() {
        let mut line = DelayLine::new(0, 1);
        assert_eq!(line.process(&[1.0, 2.0, 3.0]), vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn stereo_delay_keeps_channel_pairs_intact() {
        let mut line = DelayLine::new(1, 2); // 1 frame of delay, stereo
        let out1 = line.process(&[1.0, -1.0, 2.0, -2.0]); // two stereo frames in
        assert_eq!(out1, vec![0.0, 0.0, 1.0, -1.0]); // one silent frame, then frame 1
    }

    // ── group sink integration (demo backend, no hardware) ──────────────

    use crate::backend::demo::DemoBackend;
    use crate::engine::LiveStream;
    use crate::groups::GroupMember;

    fn demo_setup() -> (SinkManager, Arc<StreamHub>, HashMap<String, Device>, HashMap<String, SpeakerGroup>) {
        let hub = Arc::new(StreamHub::new());
        let backend: Arc<dyn AudioBackend> = Arc::new(DemoBackend::new());
        let devices: HashMap<String, Device> =
            backend.scan().unwrap().into_iter().map(|d| (d.id.clone(), d)).collect();
        let manager = SinkManager::new(
            hub.clone(),
            backend,
            Arc::new(OutputLevels::new()),
            Arc::new(PlaybackReference::new()),
        );
        let mut group = SpeakerGroup::new("g1", "Test Group");
        group.members = vec![
            GroupMember { device_id: "speakers".into(), latency_ms: 20 },
            GroupMember { device_id: "headphones".into(), latency_ms: 80 },
        ];
        let mut groups = HashMap::new();
        groups.insert("g1".to_string(), group);
        (manager, hub, devices, groups)
    }

    #[tokio::test]
    async fn group_sink_starts_and_drains_a_stream_targeting_the_group() {
        let (manager, hub, devices, groups) = demo_setup();

        let live = LiveStream::new("s1", "test-app", "g1"); // targets the GROUP id
        hub.insert(live.clone());
        live.push(&vec![0.3f32; 4800 * MIX_CHANNELS]); // 100 ms of stereo

        manager.tick(&devices, &groups).await;
        assert_eq!(manager.sink_count(), 1, "one group sink (not one per member)");

        // The group's mixer thread pulls from the stream like any sink would.
        let mut drained = false;
        for _ in 0..200 {
            if live.buffered_ms() == 0 {
                drained = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(drained, "the group sink should have consumed the stream's audio");

        manager.close_sink("g1").await;
        assert_eq!(manager.sink_count(), 0);
    }

    #[tokio::test]
    async fn group_sink_closes_when_a_member_disappears() {
        let (manager, hub, mut devices, groups) = demo_setup();

        let live = LiveStream::new("s1", "test-app", "g1");
        hub.insert(live);

        manager.tick(&devices, &groups).await;
        assert_eq!(manager.sink_count(), 1);

        // Unplug one member: the next tick notices the connected set no
        // longer matches what the sink opened with and closes it (it
        // reopens with the remaining members once the retry window allows).
        devices.remove("headphones");
        manager.tick(&devices, &groups).await;
        assert_eq!(manager.sink_count(), 0, "membership change should close the stale group sink");
    }
}