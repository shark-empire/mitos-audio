use std::sync::Arc;

use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use crate::config::AudioConfig;
use crate::errors::AudioError;
use crate::ipc::messages::{Command, Request, Response};
use crate::ipc::permissions::Permissions;
use crate::manager::state::AudioManager;

pub async fn run(config: AudioConfig) -> Result<(), AudioError> {
    let (event_tx, _) = tokio::sync::broadcast::channel(256);
    let manager = Arc::new(AudioManager::new(config.clone(), event_tx));
    let permissions = Arc::new(Permissions::current());

    let socket = std::path::Path::new(&config.socket_path);
    if let Some(dir) = socket.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            tracing::warn!(dir = ?dir, error = %e, "cannot create socket directory");
        }
    }
    if socket.exists() {
        std::fs::remove_file(socket)?;
    }
    let listener = UnixListener::bind(socket)?;
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o660));
    }
    tracing::info!(socket = %config.socket_path, "mitos-audio listening");

    loop {
        let (stream, _) = listener.accept().await?;
        let peer = stream.peer_cred().ok().map(|c| c.uid());
        let allowed = permissions.check_uid(peer);
        let manager = manager.clone();
        tokio::spawn(async move {
            if !allowed {
                tracing::warn!(uid = ?peer, "rejected connection");
                return;
            }
            if let Err(e) = handle_connection(stream, manager).await {
                tracing::debug!(error = %e, "connection ended");
            }
        });
    }
}

async fn handle_connection(
    stream: UnixStream,
    manager: Arc<AudioManager>,
) -> Result<(), AudioError> {
    let (read_half, write_half) = stream.into_split();
    let (out_tx, mut out_rx) = mpsc::channel::<String>(64);

    // Single writer task: keeps response + event lines ordered per connection.
    let writer = tokio::spawn(async move {
        let mut write_half = write_half;
        while let Some(line) = out_rx.recv().await {
            if write_half.write_all(line.as_bytes()).await.is_err() { break; }
            if write_half.write_all(b"\n").await.is_err() { break; }
        }
    });

    let mut reader = BufReader::new(read_half);
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 { break; } // client disconnected
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }

        let response = match serde_json::from_str::<Request>(trimmed) {
            Ok(request) => match request.command() {
                Ok(Command::SubscribeEvents) => {
                    let mut events = manager.subscribe();
                    let out_tx = out_tx.clone();
                    tokio::spawn(async move {
                        loop {
                            match events.recv().await {
                                Ok(event) => {
                                    if let Ok(s) = serde_json::to_string(&event) {
                                        if out_tx.send(s).await.is_err() { break; }
                                    }
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                            }
                        }
                    });
                    Response::ok(request.id, json!({ "subscribed": true }))
                }
                Ok(command) => match manager.handle(command).await {
                    Ok(result) => Response::ok(request.id, result),
                    Err(e) => Response::from_error(request.id, &e),
                },
                Err(e) => Response::from_error(request.id, &e),
            },
            Err(e) => Response::from_error(0, &AudioError::Ipc(format!("malformed request: {e}"))),
        };

        if let Ok(s) = serde_json::to_string(&response) {
            if out_tx.send(s).await.is_err() { break; }
        }
    }

    drop(out_tx);
    let _ = writer.await;
    Ok(())
}