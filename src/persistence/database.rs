//! Persisted settings (defaults, per-device volumes, profile, mic, routing
//! memory) written atomically with a 400 ms debounce, so volume-slider
//! drags don't hammer the disk and crashes never leave a torn file.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::errors::AudioError;
use crate::routing::RoutingMemory;

const DEBOUNCE: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub default_output: Option<String>,
    pub default_input: Option<String>,
    pub profile: Option<String>,
    pub mic_muted: bool,
    pub mic_gain: i32,
    pub devices: std::collections::HashMap<String, DeviceSettings>,
    pub routing: RoutingMemory,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceSettings {
    pub volume: Option<u32>,
    pub muted: Option<bool>,
}

/// Handle used by the manager; actual writes happen in a background task.
pub struct SettingsStore {
    tx: mpsc::Sender<Settings>,
}

impl SettingsStore {
    /// Load existing settings (leniently) and start the debounced writer.
    /// Must be called from within a tokio runtime.
    pub fn start(path: PathBuf) -> (Self, Settings) {
        let loaded = Self::load(&path);
        let (tx, rx) = mpsc::channel(16);
        tokio::spawn(writer(rx, path));
        (Self { tx }, loaded)
    }

    fn load(path: &Path) -> Settings {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(settings) => {
                    tracing::info!(path = %path.display(), "persisted settings loaded");
                    settings
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "corrupt state file — starting fresh");
                    Settings::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "cannot read state file — starting fresh");
                Settings::default()
            }
        }
    }

    /// Non-blocking: latest settings win; written after the debounce window.
    pub fn request_save(&self, settings: Settings) {
        let _ = self.tx.try_send(settings);
    }
}

async fn writer(mut rx: mpsc::Receiver<Settings>, path: PathBuf) {
    while let Some(mut latest) = rx.recv().await {
        // Debounce — extend the window on every update (slider drags).
        let mut deadline = tokio::time::Instant::now() + DEBOUNCE;
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => break,
                update = rx.recv() => match update {
                    Some(settings) => {
                        latest = settings;
                        deadline = tokio::time::Instant::now() + DEBOUNCE;
                    }
                    None => break,
                },
            }
        }
        if let Err(e) = write_atomic(&path, &latest) {
            tracing::warn!(path = %path.display(), error = %e, "failed to persist audio settings");
        }
    }
}

/// Write via `path.tmp` + rename — readers never observe a partial file.
fn write_atomic(path: &Path, settings: &Settings) -> Result<(), AudioError> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);

    let data = serde_json::to_string_pretty(settings)?;
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}