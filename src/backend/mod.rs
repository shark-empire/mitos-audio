pub mod backend;
pub mod demo;
#[cfg(feature = "alsa-backend")]
pub mod alsa;

pub use backend::{AudioBackend, OutputDevice};

use std::sync::Arc;

use crate::config::AudioConfig;
use crate::errors::AudioError;

#[cfg(feature = "alsa-backend")]
fn try_alsa(capture_metering: bool, latency_us: u32) -> Result<Arc<dyn AudioBackend>, AudioError> {
    Ok(Arc::new(alsa::AlsaBackend::new(capture_metering, latency_us)?))
}

#[cfg(not(feature = "alsa-backend"))]
fn try_alsa(_capture_metering: bool, _latency_us: u32) -> Result<Arc<dyn AudioBackend>, AudioError> {
    Err(AudioError::Config("this binary was built without the alsa-backend feature".into()))
}

#[cfg(feature = "alsa-backend")]
fn try_alsa_soft(capture_metering: bool, latency_us: u32) -> Option<Arc<dyn AudioBackend>> {
    alsa::AlsaBackend::new(capture_metering, latency_us)
        .ok()
        .map(|b| Arc::new(b) as Arc<dyn AudioBackend>)
}

#[cfg(not(feature = "alsa-backend"))]
fn try_alsa_soft(_capture_metering: bool, _latency_us: u32) -> Option<Arc<dyn AudioBackend>> {
    None
}

/// `"auto"` (default) uses real ALSA when compiled in and present,
/// falling back to the demo backend. `"alsa"` demands the real thing;
/// `"demo"` forces the in-memory backend (dev/CI).
pub fn create_backend(config: &AudioConfig) -> Result<Arc<dyn AudioBackend>, AudioError> {
    let latency_us = config.playback_latency_ms.saturating_mul(1000);
    match config.backend.as_str() {
        "demo" => Ok(Arc::new(demo::DemoBackend::new())),
        "alsa" => try_alsa(config.metering.capture_input, latency_us),
        _ => match try_alsa_soft(config.metering.capture_input, latency_us) {
            Some(backend) => Ok(backend),
            None => {
                tracing::warn!("no ALSA available — using demo backend");
                Ok(Arc::new(demo::DemoBackend::new()))
            }
        },
    }
}