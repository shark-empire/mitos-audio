//! Watch mitos-audio state and events.
//! Run: MITOS_AUDIO_SOCK=/tmp/mitos-dev/audio.sock \
//!      cargo run -p libmitos-audio --example volume_watch

use libmitos_audio::{AudioClient, ClientEvent};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket = std::env::var("MITOS_AUDIO_SOCK")
        .unwrap_or_else(|_| "/run/mitos/audio.sock".to_string());

    let client = AudioClient::connect(&socket).await?;
    let state = client.get_state().await?;
    println!(
        "mitos-audio v{} [backend: {}] — {} device(s), volume {}%",
        state.version, state.backend, state.devices.len(), state.master_volume
    );

    let mut events = client.subscribe();
    while let Some(event) = events.recv().await {
        match event {
            ClientEvent::Resync => {
                let s = client.get_state().await?;
                println!("[resync] {} device(s), volume {}%", s.devices.len(), s.master_volume);
            }
            ClientEvent::VolumeChanged { device, volume } => {
                println!("volume {volume}% on {device:?}");
            }
            ClientEvent::MuteChanged { muted } => {
                println!("{}", if muted { "muted" } else { "unmuted" });
            }
            ClientEvent::DefaultOutputChanged { id } => println!("output → {id}"),
            ClientEvent::DeviceAdded { id, name } => println!("+ device {id} ({name})"),
            ClientEvent::DeviceRemoved { id } => println!("- device {id}"),
            ClientEvent::MicrophoneChanged { muted, .. } => {
                println!("microphone {}", if muted { "muted" } else { "live" });
            }
            other => println!("event: {other:?}"),
        }
    }
    Ok(())
}