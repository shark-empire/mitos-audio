pub mod backend;
pub mod demo;
#[cfg(feature = "alsa-backend")]
pub mod alsa;

pub use backend::AudioBackend;

use std::sync::Arc;

use crate::config::AudioConfig;
use crate::errors::AudioError;

#[cfg(feature = "alsa-backend")]
fn try_alsa() -> Result<Arc<dyn AudioBackend>, AudioError> {
    Ok(Arc::new(alsa::AlsaBackend::new()?))
}

#[cfg(not(feature = "alsa-backend"))]
fn try_alsa() -> Result<Arc<dyn AudioBackend>, AudioError> {
    Err(AudioError::Config(
        "this binary was built without the alsa-backend feature".into(),
    ))
}

#[cfg(feature = "alsa-backend")]
fn try_alsa_soft() -> Option<Arc<dyn AudioBackend>> {
    alsa::AlsaBackend::new()
        .ok()
        .map(|b| Arc::new(b) as Arc<dyn AudioBackend>)
}

#[cfg(not(feature = "alsa-backend"))]
fn try_alsa_soft() -> Option<Arc<dyn AudioBackend>> {
    None
}

/// Select the backend according to `config.backend`:
/// `"auto"` (default) uses real ALSA when compiled in and present,
/// falling back to the demo backend otherwise. `"alsa"` demands the
/// real thing; `"demo"` forces the in-memory backend (dev/CI).
pub fn create_backend(config: &AudioConfig) -> Result<Arc<dyn AudioBackend>, AudioError> {
    match config.backend.as_str() {
        "demo" => Ok(Arc::new(demo::DemoBackend::new())),
        "alsa" => try_alsa(),
        _ => match try_alsa_soft() {
            Some(backend) => Ok(backend),
            None => {
                tracing::warn!("no ALSA available — using demo backend");
                Ok(Arc::new(demo::DemoBackend::new()))
            }
        },
    }
}