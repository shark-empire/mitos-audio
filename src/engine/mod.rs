//! The audio plane: live streams, mixing, and the data-plane protocol.
//!
//! Canonical internal format: **48 kHz, stereo, f32 interleaved**. All
//! ingest conversion happens at the data-plane edge; sinks mix in the
//! canonical format and convert once on output.

pub mod convert;
pub mod dataserver;
pub mod protocol;
pub mod sink;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MIX_RATE: u32 = 48_000;
pub const MIX_CHANNELS: usize = 2;

/// Ring capacity: 1 second of stereo f32 (samples, not frames).
const RING_CAP: usize = MIX_RATE as usize * MIX_CHANNELS;
/// Level state older than this reads as silence (no sink running).
const LEVEL_STALE_MS: u64 = 600;

/// A stream with actual audio flowing through it. Written by the
/// data-plane ingest task, steered by the manager, consumed by sink
/// mixer threads.
pub struct LiveStream {
    pub id: String,
    pub application: String,
    ring: Mutex<VecDeque<f32>>,
    volume: AtomicU32,
    muted: AtomicBool,
    device: Mutex<String>,
    starved_periods: AtomicU64,
    overflow_events: AtomicU64,
}

impl LiveStream {
    pub fn new(id: &str, application: &str, device: &str) -> Arc<Self> {
        Arc::new(Self {
            id: id.to_string(),
            application: application.to_string(),
            ring: Mutex::new(VecDeque::with_capacity(MIX_CHANNELS * 4096)),
            volume: AtomicU32::new(100),
            muted: AtomicBool::new(false),
            device: Mutex::new(device.to_string()),
            starved_periods: AtomicU64::new(0),
            overflow_events: AtomicU64::new(0),
        })
    }

    pub fn device(&self) -> String {
        self.device.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_device(&self, device: &str) {
        *self.device.lock().unwrap_or_else(|e| e.into_inner()) = device.to_string();
    }

    pub fn volume(&self) -> u32 {
        self.volume.load(Ordering::Relaxed)
    }

    pub fn set_volume(&self, volume: u32) {
        self.volume.store(volume.min(100), Ordering::Relaxed);
    }

    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    /// Pull up to `out.len() / MIX_CHANNELS` frames into `out`
    /// (interleaved stereo f32). Returns frames pulled.
    pub fn take(&self, out: &mut [f32]) -> usize {
        let mut ring = self.lock_ring();
        let frames = (ring.len() / MIX_CHANNELS).min(out.len() / MIX_CHANNELS);
        for i in 0..frames * MIX_CHANNELS {
            out[i] = ring.pop_front().unwrap_or(0.0);
        }
        frames
    }

    /// Push canonical-format samples. When the ring is full (client
    /// writing faster than realtime), the oldest half is dropped.
    pub fn push(&self, samples: &[f32]) {
        let mut ring = self.lock_ring();
        ring.extend(samples.iter().copied());
        if ring.len() > RING_CAP {
            let excess = ring.len() - RING_CAP / 2;
            ring.drain(..excess);
            self.overflow_events.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn note_starved(&self) {
        self.starved_periods.fetch_add(1, Ordering::Relaxed);
    }

    pub fn buffered_ms(&self) -> u64 {
        let frames = self.lock_ring().len() / MIX_CHANNELS;
        (frames as u64 * 1000) / MIX_RATE as u64
    }

    pub fn stats(&self) -> (u64, u64) {
        (
            self.starved_periods.load(Ordering::Relaxed),
            self.overflow_events.load(Ordering::Relaxed),
        )
    }

    fn lock_ring(&self) -> MutexGuard<'_, VecDeque<f32>> {
        self.ring.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// All live streams — the meeting point of the data-plane writers
/// (async), the manager (policy) and the sink mixer threads.
pub struct StreamHub {
    streams: Mutex<HashMap<String, Arc<LiveStream>>>,
}

impl StreamHub {
    pub fn new() -> Self {
        Self { streams: Mutex::new(HashMap::new()) }
    }

    pub fn insert(&self, live: Arc<LiveStream>) {
        self.lock().insert(live.id.clone(), live);
    }

    pub fn remove(&self, id: &str) -> Option<Arc<LiveStream>> {
        self.lock().remove(id)
    }

    pub fn get(&self, id: &str) -> Option<Arc<LiveStream>> {
        self.lock().get(id).cloned()
    }

    /// Snapshot of all live streams (cheap — clones Arcs).
    pub fn snapshot(&self) -> Vec<Arc<LiveStream>> {
        self.lock().values().cloned().collect()
    }

    /// Device ids referenced by live streams (drives sink lifecycle).
    pub fn referenced_devices(&self) -> HashSet<String> {
        self.lock().values().map(|s| s.device()).collect()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Arc<LiveStream>>> {
        self.streams.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Output level state, written by every sink's mixer thread each period
/// and read by the meter task. f32 bit patterns stored in atomics.
pub struct OutputLevels {
    rms: AtomicU32,
    peak: AtomicU32,
    clipping: AtomicBool,
    updated_ms: AtomicU64,
}

impl OutputLevels {
    pub fn new() -> Self {
        Self {
            rms: AtomicU32::new(0),
            peak: AtomicU32::new(0),
            clipping: AtomicBool::new(false),
            updated_ms: AtomicU64::new(0),
        }
    }

    pub fn update(&self, mixed: &[f32]) {
        let mut sum_sq = 0.0f64;
        let mut peak = 0.0f32;
        for &s in mixed {
            let v = s.abs();
            if v > peak {
                peak = v;
            }
            sum_sq += f64::from(s * s);
        }
        let rms = if mixed.is_empty() {
            0.0
        } else {
            (sum_sq / mixed.len() as f64).sqrt() as f32
        };

        let prev_rms = f32::from_bits(self.rms.load(Ordering::Relaxed));
        let prev_peak = f32::from_bits(self.peak.load(Ordering::Relaxed));

        self.rms.store((prev_rms * 0.6 + rms * 0.4).to_bits(), Ordering::Relaxed);
        self.peak.store(peak.max(prev_peak * 0.85).to_bits(), Ordering::Relaxed);
        self.clipping.store(peak >= 0.98, Ordering::Relaxed);
        self.updated_ms.store(now_ms(), Ordering::Relaxed);
    }

    /// (rms, peak, clipping). Zeros when stale — no sink is running.
    pub fn read(&self) -> (f32, f32, bool) {
        let now = now_ms();
        let updated = self.updated_ms.load(Ordering::Relaxed);
        if now.saturating_sub(updated) > LEVEL_STALE_MS {
            return (0.0, 0.0, false);
        }
        (
            f32::from_bits(self.rms.load(Ordering::Relaxed)),
            f32::from_bits(self.peak.load(Ordering::Relaxed)),
            self.clipping.load(Ordering::Relaxed),
        )
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}