//! Capture sources: the mirror of `engine::sink`, one input device +
//! capture thread per source device. A capture stream is a `LiveStream`
//! (the same type playback uses) registered in a *separate* hub from
//! playback's — see `manager::state`'s `capture_hub` — so a sink's
//! `hub.snapshot()` never sees a capture stream and vice versa. Direction
//! here is "device -> hub" (a capture thread pushes what it reads);
//! playback is "hub -> device" (a mixer thread pulls what apps pushed).
//! Everything else about the lifecycle (lazy open, idle close, retry
//! backoff) is deliberately identical to `engine::sink`.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::backend::{AudioBackend, InputDevice};
use crate::devices::device::Device;
use crate::errors::AudioError;
use crate::microphone::MicrophoneProcessor;

use super::{MIX_CHANNELS, PlaybackReference, StreamHub};

/// Frames per capture period (12.5 ms @ 48 kHz stereo) — matches
/// `engine::sink`'s `PERIOD_FRAMES` so a period's worth of audio is the
/// same duration on both sides of the daemon.
const PERIOD_FRAMES: usize = 600;
const IDLE_CLOSE: Duration = Duration::from_secs(5);
const OPEN_RETRY: Duration = Duration::from_secs(2);

struct SourceHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    last_used: Instant,
}

pub struct SourceManager {
    hub: Arc<StreamHub>,
    backend: Arc<dyn AudioBackend>,
    playback_reference: Arc<PlaybackReference>,
    sources: Mutex<HashMap<String, SourceHandle>>,
    last_attempt: Mutex<HashMap<String, Instant>>,
    /// One mic-processing pipeline per device, created on first reference
    /// and kept for the daemon's lifetime — same rationale as
    /// `engine::sink::SinkManager`'s `effects` registry.
    processors: Mutex<HashMap<String, Arc<Mutex<MicrophoneProcessor>>>>,
}

impl SourceManager {
    pub fn new(hub: Arc<StreamHub>, backend: Arc<dyn AudioBackend>, playback_reference: Arc<PlaybackReference>) -> Self {
        Self {
            hub,
            backend,
            playback_reference,
            sources: Mutex::new(HashMap::new()),
            last_attempt: Mutex::new(HashMap::new()),
            processors: Mutex::new(HashMap::new()),
        }
    }

    /// The shared mic-processing pipeline for `device_id`, created (all
    /// stages disabled) on first reference. `manager::state`'s microphone
    /// commands configure it; `capture_loop` applies it once per period.
    pub fn processor_for(&self, device_id: &str) -> Arc<Mutex<MicrophoneProcessor>> {
        self.lock_processors()
            .entry(device_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(MicrophoneProcessor::new())))
            .clone()
    }

    /// Lifecycle pass, same shape as `SinkManager::tick`: ensure a capture
    /// thread exists for every device referenced by a live capture stream;
    /// close idle ones and ones whose device vanished.
    pub async fn tick(&self, devices: &HashMap<String, Device>) {
        let referenced = self.hub.referenced_devices();
        let mut to_close: HashSet<String> = HashSet::new();

        {
            let mut sources = self.lock_sources();
            for id in &referenced {
                if let Some(handle) = sources.get_mut(id) {
                    handle.last_used = Instant::now();
                }
            }
            for (id, handle) in sources.iter() {
                if !devices.contains_key(id) {
                    to_close.insert(id.clone());
                } else if !referenced.contains(id) && handle.last_used.elapsed() >= IDLE_CLOSE {
                    to_close.insert(id.clone());
                }
            }
        }

        for id in &referenced {
            if self.lock_sources().contains_key(id) {
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
            match tokio::task::spawn_blocking(move || backend.open_input(&device)).await {
                Ok(Ok(input)) => {
                    let stop = Arc::new(AtomicBool::new(false));
                    let stop_thread = Arc::clone(&stop);
                    let hub = self.hub.clone();
                    let playback_reference = self.playback_reference.clone();
                    let processor = self.processor_for(id);
                    let thread_name = id.clone();
                    let join = std::thread::Builder::new()
                        .name(format!("mitos-source-{thread_name}"))
                        .spawn(move || capture_loop(thread_name, hub, input, playback_reference, processor, stop_thread))
                        .ok();
                    self.lock_sources().insert(
                        id.clone(),
                        SourceHandle { stop, join, last_used: Instant::now() },
                    );
                    tracing::info!(device = %id, "capture source started");
                }
                Ok(Err(e)) => {
                    tracing::debug!(device = %id, error = %e, "capture open failed — retrying later")
                }
                Err(e) => {
                    tracing::debug!(device = %id, error = %e, "capture open task failed")
                }
            }
        }

        for id in to_close {
            self.close_source(&id).await;
        }
    }

    /// Stop and join the capture thread for `device_id` (device removed or idle).
    pub async fn close_source(&self, device_id: &str) {
        let handle = self.lock_sources().remove(device_id);
        if let Some(mut handle) = handle {
            handle.stop.store(true, Ordering::Relaxed);
            if let Some(join) = handle.join.take() {
                let _ = tokio::task::spawn_blocking(move || {
                    let _ = join.join();
                })
                .await;
            }
            tracing::info!(device = device_id, "capture source closed");
        }
    }

    pub fn source_count(&self) -> usize {
        self.lock_sources().len()
    }

    fn lock_sources(&self) -> MutexGuard<'_, HashMap<String, SourceHandle>> {
        self.sources.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_attempts(&self) -> MutexGuard<'_, HashMap<String, Instant>> {
        self.last_attempt.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_processors(&self) -> MutexGuard<'_, HashMap<String, Arc<Mutex<MicrophoneProcessor>>>> {
        self.processors.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn capture_loop(
    device_id: String,
    hub: Arc<StreamHub>,
    mut input: Box<dyn InputDevice>,
    playback_reference: Arc<PlaybackReference>,
    processor: Arc<Mutex<MicrophoneProcessor>>,
    stop: Arc<AtomicBool>,
) {
    let mut captured = vec![0.0f32; PERIOD_FRAMES * MIX_CHANNELS];

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        match input.read(&mut captured) {
            Ok(0) => continue, // brief underrun — try again
            Ok(frames) => {
                let slice = &mut captured[..frames * MIX_CHANNELS];
                let reference = playback_reference.get(slice.len());
                processor
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .process(slice, reference.as_deref());

                for live in hub.snapshot() {
                    if live.device() == device_id {
                        live.push(slice);
                    }
                }
            }
            Err(e) => {
                tracing::warn!(device = %device_id, error = %e, "capture read failed — recovering");
                if input.recover().is_err() {
                    tracing::error!(device = %device_id, "capture device unrecoverable — exiting capture thread");
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::InputDevice;

    /// A fake input device that always returns a fixed, deterministic
    /// buffer, so the capture loop's plumbing (not any real backend) is
    /// what's under test here.
    struct FixedInput {
        value: f32,
    }

    impl InputDevice for FixedInput {
        fn read(&mut self, buf: &mut [f32]) -> Result<usize, AudioError> {
            for s in buf.iter_mut() {
                *s = self.value;
            }
            Ok(buf.len() / MIX_CHANNELS)
        }

        fn recover(&mut self) -> Result<(), AudioError> {
            Ok(())
        }
    }

    #[test]
    fn capture_loop_fans_out_to_matching_streams_only() {
        let hub = Arc::new(StreamHub::new());
        let live_a = crate::engine::LiveStream::new("cap-a", "mitos-voice", "mic-a");
        let live_b = crate::engine::LiveStream::new("cap-b", "mitos-voice", "mic-b");
        hub.insert(live_a.clone());
        hub.insert(live_b.clone());

        let input = Box::new(FixedInput { value: 0.25 });
        let playback_reference = Arc::new(PlaybackReference::new());
        let processor = Arc::new(Mutex::new(MicrophoneProcessor::new()));
        let stop = Arc::new(AtomicBool::new(false));

        // Run exactly one period, then stop, by flipping the flag from a
        // second thread once the first push lands — simplest deterministic
        // way to bound a loop meant to run forever.
        let stop_clone = stop.clone();
        let live_a_check = live_a.clone();
        let watcher = std::thread::spawn(move || {
            for _ in 0..200 {
                if live_a_check.buffered_ms() > 0 {
                    stop_clone.store(true, Ordering::Relaxed);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            stop_clone.store(true, Ordering::Relaxed);
        });

        capture_loop("mic-a".to_string(), hub, input, playback_reference, processor, stop);
        watcher.join().unwrap();

        assert!(live_a.buffered_ms() > 0, "the stream targeting mic-a should have received audio");
        assert_eq!(live_b.buffered_ms(), 0, "a stream targeting a different device must not");
    }

    #[test]
    fn processor_for_returns_the_same_instance_for_the_same_device() {
        let hub = Arc::new(StreamHub::new());
        let backend: Arc<dyn AudioBackend> = Arc::new(crate::backend::demo::DemoBackend::new());
        let manager = SourceManager::new(hub, backend, Arc::new(PlaybackReference::new()));
        let a = manager.processor_for("mic");
        let b = manager.processor_for("mic");
        assert!(Arc::ptr_eq(&a, &b));
    }
}
