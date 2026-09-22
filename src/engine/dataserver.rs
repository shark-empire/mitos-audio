//! Data-plane server: one Unix socket, binary framing, ONE stream per
//! connection (open more connections for more streams). Closing the
//! connection (or sending CLOSE) destroys the stream.

use std::sync::Arc;

use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};

use crate::errors::AudioError;
use crate::ipc::permissions::{secure_socket, Permissions};
use crate::manager::AudioManager;

use super::convert::{self, SampleFormat, StreamFormat};
use super::protocol::{self, OpenRequest, MSG_CLOSE, MSG_DATA, MSG_ERROR, MSG_OPEN, MSG_OPEN_ACK};

pub async fn run(
    manager: Arc<AudioManager>,
    path: String,
    permissions: Arc<Permissions>,
) -> Result<(), AudioError> {
    let socket = std::path::Path::new(&path);
    if let Some(dir) = socket.parent() {
        if !dir.as_os_str().is_empty() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                tracing::warn!(dir = ?dir, error = %e, "cannot create data-socket directory");
            }
        }
    }
    if socket.exists() {
        std::fs::remove_file(socket)?;
    }
    let listener = UnixListener::bind(socket)?;
    secure_socket(&path, permissions.audio_group_gid());
    tracing::info!(socket = %path, "data plane listening");

    loop {
        let (stream, _) = listener.accept().await?;
        let cred = stream.peer_cred().ok();
        if !permissions.check(cred.as_ref()) {
            tracing::warn!("data plane: rejected connection");
            continue;
        }
        let manager = manager.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, manager).await {
                tracing::debug!(error = %e, "data-plane connection ended");
            }
        });
    }
}

async fn handle_connection(
    mut stream: UnixStream,
    manager: Arc<AudioManager>,
) -> Result<(), AudioError> {
    let mut current: Option<(String, StreamFormat)> = None;

    loop {
        let frame = match protocol::read_frame(&mut stream).await {
            Ok(frame) => frame,
            Err(_) => break, // client gone — destroy below
        };

        match frame.kind {
            MSG_OPEN if current.is_none() => match open_stream(&manager, &frame.payload).await {
                Ok((stream_id, fmt)) => {
                    let payload = json!({ "stream_id": stream_id }).to_string();
                    protocol::write_frame(&mut stream, MSG_OPEN_ACK, payload.as_bytes()).await?;
                    current = Some((stream_id, fmt));
                }
                Err(e) => send_error(&mut stream, e.code(), &e.to_string()).await,
            },

            MSG_OPEN => {
                send_error(&mut stream, "ipc-error", "a stream is already open on this connection").await;
            }

            MSG_DATA => {
                if let Some((id, fmt)) = &current {
                    if !frame.payload.is_empty() {
                        let samples = convert::ingest(fmt, &frame.payload);
                        manager.push_stream_data(id, &samples);
                    }
                } else {
                    send_error(&mut stream, "ipc-error", "DATA before OPEN").await;
                    break;
                }
            }

            MSG_CLOSE => {
                let _ = protocol::write_frame(&mut stream, super::protocol::MSG_CLOSED, b"{}").await;
                let _ = stream.shutdown().await;
                break;
            }

            _ => {
                send_error(&mut stream, "ipc-error", "unknown frame type").await;
                break;
            }
        }
    }

    if let Some((id, _)) = current {
        let _ = manager.destroy_live_stream(&id).await;
    }
    Ok(())
}

async fn open_stream(
    manager: &AudioManager,
    payload: &[u8],
) -> Result<(String, StreamFormat), AudioError> {
    let request: OpenRequest = serde_json::from_slice(payload)
        .map_err(|e| AudioError::Ipc(format!("malformed OPEN: {e}")))?;
    let fmt = StreamFormat {
        sample_rate: request.sample_rate,
        channels: request.channels,
        format: SampleFormat::parse(&request.format)?,
    };
    convert::validate(fmt.sample_rate, fmt.channels)?;
    let stream = manager
        .create_live_stream(&request.application, &fmt, request.device.as_deref())
        .await?;
    Ok((stream.id, fmt))
}

async fn send_error(stream: &mut UnixStream, code: &str, message: &str) {
    let payload = json!({ "code": code, "message": message }).to_string();
    let _ = protocol::write_frame(stream, MSG_ERROR, payload.as_bytes()).await;
}