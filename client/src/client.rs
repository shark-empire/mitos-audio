use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio::sync::{mpsc, Mutex};

use crate::connection::Connection;
use crate::error::ClientError;
use crate::events::{ClientEvent, EventStream};

/// Client tuning knobs.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Daemon socket (env `MITOS_AUDIO_SOCK` overrides the default).
    pub socket_path: PathBuf,
    /// Initial reconnect delay for the event monitor.
    pub retry_initial: Duration,
    /// Reconnect delay ceiling (exponential backoff).
    pub retry_max: Duration,
}

impl Default for ClientConfig {
    fn default() -> Self {
        let socket_path = std::env::var("MITOS_AUDIO_SOCK")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/run/mitos/audio.sock"));
        Self {
            socket_path,
            retry_initial: Duration::from_millis(250),
            retry_max: Duration::from_secs(8),
        }
    }
}

/// Typed mitos-audio client.
///
/// `Send + Sync` — share one instance (`Arc<AudioClient>`) across your whole
/// GUI. Requests serialize internally; connection-level failures get one
/// transparent reconnect+retry.
pub struct AudioClient {
    config: ClientConfig,
    conn: Mutex<Option<Connection>>,
    next_id: AtomicU64,
}

impl AudioClient {
    /// Connect eagerly and verify the daemon answers `Ping`.
    pub async fn connect(socket: impl AsRef<std::path::Path>) -> Result<Self, ClientError> {
        let mut config = ClientConfig::default();
        config.socket_path = socket.as_ref().to_path_buf();
        let client = Self::with_config(config);
        client.ping().await?;
        Ok(client)
    }

    /// Lazy client — connects on first use (and reconnects as needed).
    pub fn new() -> Self {
        Self::with_config(ClientConfig::default())
    }

    pub fn with_config(config: ClientConfig) -> Self {
        Self { config, conn: Mutex::new(None), next_id: AtomicU64::new(1) }
    }

    pub fn socket_path(&self) -> &std::path::Path {
        &self.config.socket_path
    }
    
        // ── levels (dedicated 10 Hz subscription) ───────────────────────────

    /// Live level meters. Never fails — the monitor reconnects with
    /// backoff forever. Frames only arrive while this stream is alive
    /// (the daemon parks hardware metering when nobody listens).
    pub fn subscribe_levels(&self) -> crate::LevelStream {
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let config = self.config.clone();
        tokio::spawn(crate::levels::level_monitor(config, tx));
        crate::LevelStream { rx }
    }

    // ── streams (interim application registration) ──────────────────────

    pub async fn create_stream(
        &self,
        application: &str,
        device: Option<&str>,
        kind: Option<&str>,
    ) -> Result<crate::types::StreamInfo, ClientError> {
        #[derive(serde::Deserialize)]
        struct Wrapper {
            stream: crate::types::StreamInfo,
        }
        let mut params = json!({ "application": application });
        if let Some(d) = device {
            params["device"] = json!(d);
        }
        if let Some(k) = kind {
            params["kind"] = json!(k);
        }
        let wrapper: Wrapper = self.typed("CreateStream", params).await?;
        Ok(wrapper.stream)
    }

    pub async fn destroy_stream(&self, stream_id: &str) -> Result<(), ClientError> {
        self.call("DestroyStream", json!({ "stream_id": stream_id })).await?;
        Ok(())
    }

    // ── routing ─────────────────────────────────────────────────────────

    /// Re-read the daemon's routing.toml; returns the rule count.
    pub async fn reload_routing(&self) -> Result<usize, ClientError> {
        #[derive(serde::Deserialize)]
        struct Wrapper {
            rules: usize,
        }
        let wrapper: Wrapper = self.typed("ReloadRouting", json!({})).await?;
        Ok(wrapper.rules)
    }

    // ── plumbing ────────────────────────────────────────────────────────

    async fn call(&self, command: &str, params: Value) -> Result<Value, ClientError> {
        let mut guard = self.conn.lock().await;
        if guard.is_none() {
            *guard = Some(Connection::open(&self.config.socket_path).await?);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        let result = guard.as_mut().unwrap().request(id, command, params.clone()).await;
        match result {
            Ok(value) => Ok(value),
            // Connection-level failure: reconnect once, retry transparently.
            Err(ClientError::Io(_)) | Err(ClientError::Disconnected) => {
                tracing::debug!("connection lost — reconnecting once");
                *guard = None;
                let mut conn = Connection::open(&self.config.socket_path).await?;
                let value = conn.request(id, command, params).await?;
                *guard = Some(conn);
                Ok(value)
            }
            Err(e) => Err(e),
        }
    }

    async fn typed<T: DeserializeOwned>(
        &self,
        command: &str,
        params: Value,
    ) -> Result<T, ClientError> {
        let value = self.call(command, params).await?;
        Ok(serde_json::from_value(value)?)
    }

    // ── commands ────────────────────────────────────────────────────────

    pub async fn ping(&self) -> Result<(), ClientError> {
        self.call("Ping", json!({})).await?;
        Ok(())
    }

    pub async fn get_state(&self) -> Result<crate::types::SystemState, ClientError> {
        self.typed("GetState", json!({})).await
    }

    pub async fn list_devices(&self) -> Result<crate::types::DeviceList, ClientError> {
        self.typed("ListDevices", json!({})).await
    }

    pub async fn rescan(&self) -> Result<crate::types::DeviceList, ClientError> {
        self.typed("Rescan", json!({})).await
    }

    pub async fn get_device(&self, id: &str) -> Result<crate::types::DeviceInfo, ClientError> {
        self.typed("GetDevice", json!({ "id": id })).await
    }

    pub async fn set_default_output(&self, id: &str) -> Result<(), ClientError> {
        self.call("SetDefaultOutput", json!({ "id": id })).await?;
        Ok(())
    }

    pub async fn set_default_input(&self, id: &str) -> Result<(), ClientError> {
        self.call("SetDefaultInput", json!({ "id": id })).await?;
        Ok(())
    }

    /// Master volume (= default output device's volume).
    pub async fn volume(&self) -> Result<crate::types::VolumeInfo, ClientError> {
        self.typed("GetVolume", json!({})).await
    }

    pub async fn set_volume(&self, volume: u32) -> Result<crate::types::VolumeChangeResult, ClientError> {
        self.typed("SetVolume", json!({ "volume": volume })).await
    }

    pub async fn device_volume(&self, device: &str) -> Result<crate::types::VolumeInfo, ClientError> {
        self.typed("GetVolume", json!({ "device": device })).await
    }

    pub async fn set_device_volume(
        &self,
        device: &str,
        volume: u32,
    ) -> Result<crate::types::VolumeChangeResult, ClientError> {
        self.typed("SetVolume", json!({ "device": device, "volume": volume })).await
    }

    pub async fn mute(&self) -> Result<(), ClientError> {
        self.call("Mute", json!({})).await?;
        Ok(())
    }

    pub async fn unmute(&self) -> Result<(), ClientError> {
        self.call("Unmute", json!({})).await?;
        Ok(())
    }

    pub async fn list_streams(&self) -> Result<Vec<crate::types::StreamInfo>, ClientError> {
        #[derive(serde::Deserialize)]
        struct Wrapper {
            streams: Vec<crate::types::StreamInfo>,
        }
        let wrapper: Wrapper = self.typed("ListStreams", json!({})).await?;
        Ok(wrapper.streams)
    }

    pub async fn set_stream_volume(&self, stream_id: &str, volume: u32) -> Result<(), ClientError> {
        self.call("SetStreamVolume", json!({ "stream_id": stream_id, "volume": volume })).await?;
        Ok(())
    }

    pub async fn set_stream_mute(&self, stream_id: &str, mute: bool) -> Result<(), ClientError> {
        self.call("SetStreamMute", json!({ "stream_id": stream_id, "mute": mute })).await?;
        Ok(())
    }

    pub async fn move_stream(&self, stream_id: &str, device_id: &str) -> Result<(), ClientError> {
        self.call("MoveStream", json!({ "stream_id": stream_id, "device_id": device_id })).await?;
        Ok(())
    }

    pub async fn list_profiles(&self) -> Result<crate::types::ProfileList, ClientError> {
        self.typed("ListProfiles", json!({})).await
    }

    pub async fn set_profile(&self, profile: &str) -> Result<(), ClientError> {
        self.call("SetProfile", json!({ "profile": profile })).await?;
        Ok(())
    }

    pub async fn microphone(&self) -> Result<crate::types::MicrophoneStatus, ClientError> {
        self.typed("GetMicrophone", json!({})).await
    }

    pub async fn set_microphone_gain(&self, gain_db: i32) -> Result<(), ClientError> {
        self.call("SetMicrophoneGain", json!({ "gain": gain_db })).await?;
        Ok(())
    }

    pub async fn mute_microphone(&self) -> Result<(), ClientError> {
        self.call("MuteMicrophone", json!({ "mute": true })).await?;
        Ok(())
    }

    pub async fn unmute_microphone(&self) -> Result<(), ClientError> {
        self.call("MuteMicrophone", json!({ "mute": false })).await?;
        Ok(())
    }

    pub async fn levels(&self) -> Result<crate::types::Levels, ClientError> {
        self.typed("GetLevels", json!({})).await
    }

    // ── events ──────────────────────────────────────────────────────────

    /// Subscribe to daemon events on a dedicated background connection.
    ///
    /// Must be called from within a tokio runtime. Never fails: the monitor
    /// task connects (and reconnects forever with backoff) in the background.
    /// The **first** event is `ClientEvent::Resync` once connected — call
    /// `get_state()` when you receive it. The monitor stops when the
    /// `EventStream` is dropped.
    pub fn subscribe(&self) -> EventStream {
        let (tx, rx) = mpsc::channel(64);
        let config = self.config.clone();
        tokio::spawn(monitor(config, tx));
        EventStream { rx }
    }
}

/// Background monitor: dedicated connection, eternal reconnect, resync signal.
async fn monitor(config: ClientConfig, tx: mpsc::Sender<ClientEvent>) {
    let mut backoff = config.retry_initial;
    loop {
        // Connect + subscribe.
        let mut conn = match Connection::open(&config.socket_path).await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::debug!(error = %e, "monitor: connect failed");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(config.retry_max);
                continue;
            }
        };
        if conn.request(0, "SubscribeEvents", json!({})).await.is_err() {
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(config.retry_max);
            continue;
        }
        backoff = config.retry_initial;

        // Connected (or reconnected) — consumers should refetch state.
        if tx.send(ClientEvent::Resync).await.is_err() {
            return; // receiver dropped — stop the monitor
        }

        // Stream events until the connection drops.
        loop {
            match conn.read_line().await {
                Ok(line) => {
                    let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
                    if let Some(event) = ClientEvent::from_wire(&value) {
                        if tx.send(event).await.is_err() {
                            return;
                        }
                    }
                }
                Err(_) => break, // disconnected — outer loop reconnects
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(config.retry_max);
    }
}