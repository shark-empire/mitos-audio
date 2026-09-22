libmitos-audio

Typed async client for the mitos-audio daemon. Built formitos-gui and mitos-settings, usable from any Rust application.

Add it

[dependencies]libmitos-audio = "0.1"tokio = { version = "1", features = ["rt", "net", "macros"] }
Quick start

use libmitos_audio::AudioClient;#[tokio::main]async fn main() -> Result<(), libmitos_audio::ClientError> {    let client = AudioClient::connect("/run/mitos/audio.sock").await?; // or env MITOS_AUDIO_SOCK    let state = client.get_state().await?;    println!("{} device(s), volume {}%", state.devices.len(), state.master_volume);    client.set_volume(75).await?;    client.set_default_output(&state.default_output.clone().unwrap()).await?;    Ok(())}
API tour

Area	Methods
Devices	list_devices, get_device, rescan, set_default_output, set_default_input
Volume	volume, set_volume, device_volume, set_device_volume, mute, unmute
Streams	list_streams, set_stream_volume, set_stream_mute, move_stream
Profiles	list_profiles, set_profile
Microphone	microphone, set_microphone_gain, mute_microphone, unmute_microphone
Monitoring	levels, subscribe
Master volume is the default output device's volume. SetVolume resultscarry hw_applied: bool — false means the device has no hardware volumecontrol (state is tracked anyway).

The GUI pattern (Resync)

let client = Arc::new(AudioClient::connect(SOCK).await?);// One-shot model build.let state = client.get_state().await?;build_ui(&state);// Live updates, forever, across daemon restarts.let mut events = client.subscribe();loop {    match events.recv().await? {        ClientEvent::Resync => {                    // (re)connected — refetch!            let s = client.get_state().await?;            rebuild_ui(&s);        }        ClientEvent::VolumeChanged { volume, .. } => volume_slider.set(volume),        ClientEvent::DeviceAdded { id, name } => device_list.add(id, name),        ClientEvent::DefaultOutputChanged { id } => osd.notify_output(id),        ClientEvent::MuteChanged { muted } => osd.show_mute(muted),        _ => {}    }}
Why this works:

subscribe() never fails — the monitor connects and reconnects withexponential backoff (250 ms → 8 s) in the background.
ClientEvent::Resync fires after every (re)connect — your single cue torefetch state, so you never render stale data after a daemon restart.
AudioClient is Send + Sync; share one Arc<AudioClient> app-wide.Requests auto-reconnect with one transparent retry.
Unknown events from newer daemons arrive as ClientEvent::Unknown —ignore and move on (protocol compatibility rule).
Errors

ClientError::Io (socket trouble), ::Disconnected, ::Protocol(malformed data), ::Serialization, and ::Daemon { code, message } —stable codes like device-not-found, invalid-volume, no-output-device.

Run the example

cargo run -p libmitos-audio --example volume_watchMITOS_AUDIO_SOCK=/tmp/mitos-dev/audio.sock cargo run -p libmitos-audio --example volume_watch