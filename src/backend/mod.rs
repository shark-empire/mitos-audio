pub mod backend;
pub mod demo;
#[cfg(feature = "alsa-backend")]
pub mod alsa;

pub use backend::AudioBackend;

use std::sync::Arc;

use crate::config::AudioConfig;
use crate::errors::AudioError;

#[cfg(feature = "alsa-backend")]
fn try_alsa(capture_metering: bool) -> Result<Arc<dyn AudioBackend>, AudioError> {
    Ok(Arc::new(alsa::AlsaBackend::new(capture_metering)?))
}

#[cfg(not(feature = "alsa-backend"))]
fn try_alsa(_capture_metering: bool) -> Result<Arc<dyn AudioBackend>, AudioError> {
    Err(AudioError::Config(
        "this binary was built without the alsa-backend feature".into(),
    ))
}

#[cfg(feature = "alsa-backend")]
fn try_alsa_soft(capture_metering: bool) -> Option<Arc<dyn AudioBackend>> {
    alsa::AlsaBackend::new(capture_metering)
        .ok()
        .map(|b| Arc::new(b) as Arc<dyn AudioBackend>)
}

#[cfg(not(feature = "alsa-backend"))]
fn try_alsa_soft(_capture_metering: bool) -> Option<Arc<dyn AudioBackend>> {
    None
}

/// `"auto"` (default) uses real ALSA when compiled in and present,
/// falling back to the demo backend. `"alsa"` demands the real thing;
/// `"demo"` forces the in-memory backend (dev/CI).
pub fn create_backend(config: &AudioConfig) -> Result<Arc<dyn AudioBackend>, AudioError> {
    match config.backend.as_str() {
        "demo" => Ok(Arc::new(demo::DemoBackend::new())),
        "alsa" => try_alsa(config.metering.capture_input),
        _ => match try_alsa_soft(config.metering.capture_input) {
            Some(backend) => Ok(backend),
            None => {
                tracing::warn!("no ALSA available — using demo backend");
                Ok(Arc::new(demo::DemoBackend::new()))
            }
        },
    }
}