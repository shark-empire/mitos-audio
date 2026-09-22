//! Dedicated level-meter subscription: reconnects forever, parses
//! `{"event":"LevelChanged","data":{...}}` lines into typed frames.

use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::connection::Connection;
use crate::types::LevelFrame;

/// Receive half of the level monitor. Stops when dropped.
pub struct LevelStream {
    rx: mpsc::Receiver<LevelFrame>,
}

impl LevelStream {
    /// Await the next frame. `None` once the client is dropped.
    pub async fn recv(&mut self) -> Option<LevelFrame> {
        self.rx.recv().await
    }
}

pub(crate) async fn level_monitor(
    config: crate::ClientConfig,
    tx: mpsc::Sender<LevelFrame>,
) {
    let mut backoff = config.retry_initial;
    loop {
        let mut conn = match Connection::open(&config.socket_path).await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::debug!(error = %e, "level monitor: connect failed");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(config.retry_max);
                continue;
            }
        };
        if conn.request(0, "SubscribeLevels", json!({})).await.is_err() {
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(config.retry_max);
            continue;
        }
        backoff = config.retry_initial;

        loop {
            match conn.read_line().await {
                Ok(line) => {
                    let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
                    if value.get("event").and_then(Value::as_str) != Some("LevelChanged") {
                        continue;
                    }
                    if let Some(data) = value.get("data") {
                        if let Ok(frame) = serde_json::from_value::<LevelFrame>(data.clone()) {
                            if tx.send(frame).await.is_err() {
                                return; // receiver dropped — stop the monitor
                            }
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