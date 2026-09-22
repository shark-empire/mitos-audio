use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};

use crate::backend::AudioBackend;
use crate::config::{AudioConfig, MeteringConfig};
use crate::engine::dataserver;
use crate::errors::AudioError;
use crate::ipc::messages::{Command, Event, Request, Response};
use crate::ipc::permissions::{secure_socket, Permissions};
use crate::manager::AudioManager;
use crate::monitoring::LevelFrame;
use crate::persistence::SettingsStore;

pub async fn run(mut config: AudioConfig) -> Result<(), AudioError> {
    // Dev convenience: derive the data socket when only socket_path is set.
    if config.data_socket_path == "/run/mitos/audio-data.sock"
        && config.socket_path != "/run/mitos/audio.sock"
    {
        config.data_socket_path =
            crate::engine::protocol::derive_data_socket(&config.socket_path);
    }

    let (event_tx, _) = broadcast::channel::<Event>(256);
    let (levels_tx, _) = broadcast::channel::<Event>(64);

    let backend = crate::backend::create_backend(&config)?;
    tracing::info!(backend = backend.name(), "audio backend selected");

    let permissions = Arc::new(Permissions::current());

    let (store, saved) = if config.persist {
        let (store, saved) = SettingsStore::start(std::path::PathBuf::from(&config.state_path));
        tracing::info!(path = %config.state_path, "persistence enabled");
        (Some(store), saved)
    } else {
        tracing::info!("persistence disabled (persist = false)");
        (None, Default::default())
    };

    let manager = Arc::new(AudioManager::new(config.clone(), backend.clone(), event_tx, store));
    tracing::info!(rules = manager.routing().rule_count(), "routing rules loaded");

    if let Err(e) = manager.refresh().await {
        tracing::error!(error = %e, "initial device scan failed (continuing)");
    }
    if config.persist {
        manager.restore(saved).await;
    }

    // ── hotplug: udev watcher (instant) ─────────────────────────────────
    if config.hotplug.udev {
        #[cfg(feature = "udev-hotplug")]
        hotplug::spawn_udev_watcher(manager.clone());
        #[cfg(not(feature = "udev-hotplug"))]
        tracing::warn!(
            "config requests udev hotplug, but this build lacks the 'udev-hotplug' feature; relying on polling"
        );
    }

    // ── hotplug: periodic poll (safety net + external mixer changes) ────
    if config.hotplug.poll_secs > 0 {
        let manager = manager.clone();
        let period = Duration::from_secs(config.hotplug.poll_secs);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(period);
            tick.tick().await;
            loop {
                tick.tick().await;
                if let Err(e) = manager.refresh().await {
                    tracing::debug!(error = %e, "periodic rescan failed");
                }
            }
        });
    }

    // ── data plane (binary audio frames) ────────────────────────────────
    {
        let manager = manager.clone();
        let path = config.data_socket_path.clone();
        let perms = permissions.clone();
        tokio::spawn(async move {
            if let Err(e) = dataserver::run(manager, path, perms).await {
                tracing::error!(error = %e, "data-plane server failed");
            }
        });
    }

    // ── sink lifecycle: open/close output devices as streams need them ──
    {
        let manager = manager.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(500));
            tick.tick().await;
            loop {
                tick.tick().await;
                let devices = manager.devices_snapshot().await;
                manager.sinks().tick(&devices).await;
            }
        });
    }

    // ── level metering (~10 Hz, only while subscribers exist) ───────────
    spawn_meter_task(manager.clone(), backend.clone(), levels_tx.clone(), config.metering.clone());

    // ── control socket (JSONL) ──────────────────────────────────────────
    let socket = std::path::Path::new(&config.socket_path);
    if let Some(dir) = socket.parent() {
        if !dir.as_os_str().is_empty() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                tracing::warn!(dir = ?dir, error = %e, "cannot create socket directory");
            }
        }
    }
    if socket.exists() {
        std::fs::remove_file(socket)?;
    }
    let listener = UnixListener::bind(socket)?;
    secure_socket(&config.socket_path, permissions.audio_group_gid());
    tracing::info!(socket = %config.socket_path, "mitos-audio listening");

    loop {
        let (stream, _) = listener.accept().await?;
        let cred = stream.peer_cred().ok();
        if !permissions.check(cred.as_ref()) {
            tracing::warn!("control plane: rejected connection");
            continue;
        }
        let manager = manager.clone();
        let levels_tx = levels_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, manager, levels_tx).await {
                tracing::debug!(error = %e, "connection ended");
            }
        });
    }
}

fn spawn_meter_task(
    manager: Arc<AudioManager>,
    backend: Arc<dyn AudioBackend>,
    levels_tx: broadcast::Sender<Event>,
    cfg: MeteringConfig,
) {
    let out_levels = manager.output_levels();
    tokio::spawn(async move {
        let interval = Duration::from_millis(cfg.interval_ms.clamp(20, 1000));
        let mut tick = tokio::time::interval(interval);
        tick.tick().await;
        loop {
            tick.tick().await;
            if !cfg.enabled {
                continue;
            }
            if levels_tx.receiver_count() == 0 {
                backend.set_metering_active(false);
                continue;
            }
            backend.set_metering_active(true);
            // Output: measured by the sink mixers. Input: from the backend
            // (ALSA capture meter / demo noise).
            let (out_rms, out_peak, out_clip) = out_levels.read();
            let b = backend.clone();
            match tokio::task::spawn_blocking(move || b.levels()).await {
                Ok(Ok(input)) => {
                    let frame = LevelFrame {
                        output_level: out_rms,
                        input_level: input.input_level,
                        peak: out_peak.max(input.peak),
                        clipping: out_clip || input.clipping,
                    };
                    manager.update_levels(frame).await;
                    let _ = levels_tx.send(Event::LevelChanged {
                        output_level: frame.output_level,
                        input_level: frame.input_level,
                        peak: frame.peak,
                        clipping: frame.clipping,
                    });
                }
                Ok(Err(e)) => tracing::debug!(error = %e, "level read failed"),
                Err(e) => tracing::debug!(error = %e, "meter task join failed"),
            }
        }
    });
}

async fn handle_connection(
    stream: UnixStream,
    manager: Arc<AudioManager>,
    levels_tx: broadcast::Sender<Event>,
) -> Result<(), AudioError> {
    let (read_half, write_half) = stream.into_split();
    let (out_tx, mut out_rx) = mpsc::channel::<String>(64);

    let writer = tokio::spawn(async move {
        let mut write_half = write_half;
        while let Some(line) = out_rx.recv().await {
            if write_half.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            if write_half.write_all(b"\n").await.is_err() {
                break;
            }
        }
    });

    let mut reader = BufReader::new(read_half);
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

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
                                        if out_tx.send(s).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                                Err(broadcast::error::RecvError::Closed) => break,
                            }
                        }
                    });
                    Response::ok(request.id, json!({ "subscribed": true }))
                }
                Ok(Command::SubscribeLevels) => {
                    let mut levels = levels_tx.subscribe();
                    let out_tx = out_tx.clone();
                    tokio::spawn(async move {
                        loop {
                            match levels.recv().await {
                                Ok(event) => {
                                    if let Ok(s) = serde_json::to_string(&event) {
                                        if out_tx.send(s).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                                Err(broadcast::error::RecvError::Closed) => break,
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
            Err(e) => {
                Response::from_error(0, &AudioError::Ipc(format!("malformed request: {e}")))
            }
        };

        if let Ok(s) = serde_json::to_string(&response) {
            if out_tx.send(s).await.is_err() {
                break;
            }
        }
    }

    drop(out_tx);
    let _ = writer.await;
    Ok(())
}