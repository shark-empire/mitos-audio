//! Playback over the data plane. Wire constants mirror
//! `mitos-audio::engine::protocol` (docs/audio-plane.md is the contract).

use std::path::Path;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::error::ClientError;

const MSG_OPEN: u8 = 0x01;
const MSG_DATA: u8 = 0x02;
const MSG_CLOSE: u8 = 0x03;
const MSG_OPEN_ACK: u8 = 0x81;
const MSG_ERROR: u8 = 0x82;

/// One playback stream: one data-plane connection.
pub struct PlaybackStream {
    socket: UnixStream,
    stream_id: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub format: String,
}

impl PlaybackStream {
    pub async fn open(
        data_socket: impl AsRef<Path>,
        application: &str,
        sample_rate: u32,
        channels: u16,
        format: &str,
    ) -> Result<Self, ClientError> {
        let mut socket = UnixStream::connect(data_socket.as_ref()).await?;
        let payload = serde_json::json!({
            "application": application,
            "sample_rate": sample_rate,
            "channels": channels,
            "format": format,
        });
        send(&mut socket, MSG_OPEN, payload.to_string().as_bytes()).await?;
        let (kind, body) = recv(&mut socket).await?;
        match kind {
            MSG_OPEN_ACK => {
                let v: Value = serde_json::from_slice(&body)?;
                let id = v["stream_id"]
                    .as_str()
                    .ok_or_else(|| ClientError::Protocol("OPEN_ACK without stream_id".into()))?
                    .to_string();
                Ok(Self {
                    socket,
                    stream_id: id,
                    sample_rate,
                    channels,
                    format: format.to_string(),
                })
            }
            MSG_ERROR => {
                let v: Value = serde_json::from_slice(&body)?;
                Err(ClientError::Daemon {
                    code: v["code"].as_str().unwrap_or("unknown").to_string(),
                    message: v["message"].as_str().unwrap_or("unknown").to_string(),
                })
            }
            other => Err(ClientError::Protocol(format!("unexpected frame 0x{other:02x}"))),
        }
    }

    pub fn stream_id(&self) -> &str {
        &self.stream_id
    }

    /// Raw PCM bytes in the negotiated format (interleaved).
    pub async fn write_pcm(&mut self, bytes: &[u8]) -> Result<(), ClientError> {
        send(&mut self.socket, MSG_DATA, bytes).await
    }

    /// Interleaved i16 samples (for s16le streams).
    pub async fn write_i16(&mut self, samples: &[i16]) -> Result<(), ClientError> {
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for s in samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        self.write_pcm(&bytes).await
    }

    /// Graceful close. (The daemon also cleans up if the socket drops.)
    pub async fn close(mut self) -> Result<(), ClientError> {
        send(&mut self.socket, MSG_CLOSE, &[]).await?;
        let _ = self.socket.shutdown().await;
        Ok(())
    }
}

async fn send(socket: &mut UnixStream, kind: u8, payload: &[u8]) -> Result<(), ClientError> {
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.push(kind);
    frame.extend_from_slice(payload);
    socket.write_all(&frame).await?;
    Ok(())
}

async fn recv(socket: &mut UnixStream) -> Result<(u8, Vec<u8>), ClientError> {
    let mut header = [0u8; 5];
    socket.read_exact(&mut header).await?;
    let len = u32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize;
    if len > (1 << 20) {
        return Err(ClientError::Protocol("frame too large".into()));
    }
    let mut body = vec![0u8; len];
    socket.read_exact(&mut body).await?;
    Ok((header[4], body))
}