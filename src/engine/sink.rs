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
use crate::errors::AudioError;

use super::{MIX_CHANNELS, OutputLevels, StreamHub};

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
}

struct SinkHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    last_used: Instant,
}

impl SinkManager {
    pub fn new(
        hub: Arc<StreamHub>,
        backend: Arc<dyn AudioBackend>,
        levels: Arc<OutputLevels>,
    ) -> Self {
        Self {
            hub,
            backend,
            levels,
            sinks: Mutex::new(HashMap::new()),
            last_attempt: Mutex::new(HashMap::new()),
        }
    }

    /// Lifecycle pass (~every 500 ms): ensure sinks exist for every
    /// referenced device; close idle sinks and sinks whose device
    /// vanished. `devices` is the current device registry.
    pub async fn tick(&self, devices: &HashMap<String, Device>) {
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
                if !devices.contains_key(id) {
                    to_close.insert(id.clone());
                } else if !referenced.contains(id) && handle.last_used.elapsed() >= IDLE_CLOSE {
                    to_close.insert(id.clone());
                }
            }
        }

        for id in &referenced {
            if self.lock_sinks().contains_key(id) {
                continue;
            }
            let Some(device) = devices.get(id) else { continue };

            {
                let mut attempts = self.lock_attempts();
                let last = attempts.get(id).map(|t| t.elapsed()).unwrap_or(Duration::MAX);
                if last < OPEN_RETRY {
                    continue;
                }
                attempts.insert(id.clone(), Instant::now());
            }

            let device = device.clone();
            let backend = self.backend.clone();
            match tokio::task::spawn_blocking(move || backend.open_output(&device)).await {
                Ok(Ok(out)) => {
                    let stop = Arc::new(AtomicBool::new(false));
                    let stop_thread = Arc::clone(&stop);
                    let hub = self.hub.clone();
                    let levels = self.levels.clone();
                    let thread_name = id.clone();
                    let join = std::thread::Builder::new()
                        .name(format!("mitos-sink-{thread_name}"))
                        .spawn(move || mixer_loop(thread_name, hub, out, levels, stop_thread))
                        .ok();
                    self.lock_sinks().insert(
                        id.clone(),
                        SinkHandle { stop, join, last_used: Instant::now() },
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
    stop: Arc<AtomicBool>,
) {
    let mut mix = vec![0.0f32; PERIOD_FRAMES * MIX_CHANNELS];
    let mut read = vec![0.0f32; PERIOD_FRAMES * MIX_CHANNELS];

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        mix.iter_mut().for_each(|s| *s = 0.0);

        for live in hub.snapshot() {
            if live.device() != device_id {
                continue;
            }
            let frames = live.take(&mut read);
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