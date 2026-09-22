//! udev-based hotplug detection — instant, replaces polling as primary.
//!
//! Any "sound" subsystem uevent (USB plug, HDMI monitor attach, Bluetooth
//! profile registration once mitos-bluetooth exists) triggers one debounced
//! full rescan. The periodic poll in the daemon stays as a safety net and
//! to pick up external mixer changes, which udev cannot see.

use std::sync::Arc;
use std::time::Duration;

use crate::manager::AudioManager;

const BURST_WINDOW: Duration = Duration::from_millis(400);

/// Spawn the watcher. Returns immediately; failures are logged and the
/// daemon's polling fallback remains active.
#[cfg(feature = "udev-hotplug")]
pub fn spawn_udev_watcher(manager: Arc<AudioManager>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    std::thread::Builder::new()
        .name("mitos-audio-udev".into())
        .spawn(move || {
            let socket = match udev::MonitorBuilder::new()
                .and_then(|b| b.match_subsystem("sound"))
                .and_then(|b| b.listen())
            {
                Ok(socket) => socket,
                Err(e) => {
                    tracing::warn!(error = %e, "udev monitor unavailable — falling back to polling");
                    return;
                }
            };
            tracing::info!("udev hotplug watcher active (subsystem: sound)");

            // udev 0.9 API — on older crate versions use `socket.iter()`.
            // Payload doesn't matter: the debounced refresh does a full scan.
            for _event in socket.iter(None) {
                if tx.send(()).is_err() {
                    return; // daemon shutting down
                }
            }
        })
        .expect("cannot spawn udev watcher thread");

    tokio::spawn(async move {
        while rx.recv().await.is_some() {
            // Debounce: collect the uevent burst (kernel emits several per
            // plug: card, pcm, control…), then rescan exactly once.
            let deadline = tokio::time::Instant::now() + BURST_WINDOW;
            loop {
                tokio::select! {
                    _ = tokio::time::sleep_until(deadline) => break,
                    Some(_) = rx.recv() => {}
                    else => break,
                }
            }
            tracing::debug!("udev: sound subsystem change — rescanning");
            let _ = manager.refresh().await;
        }
    });
}