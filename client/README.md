# libmitos-audio

Typed async client for the mitos-audio daemon. Built for mitos-gui and
mitos-settings, usable from any Rust application.

## Add it

```toml
[dependencies]
libmitos-audio = "0.3"
tokio = { version = "1", features = ["rt", "net", "macros"] }
```

## Quick start

```rust
use libmitos_audio::AudioClient;

#[tokio::main]
async fn main() -> Result<(), libmitos_audio::ClientError> {
    let client = AudioClient::connect("/run/mitos/audio.sock").await?; // or env MITOS_AUDIO_SOCK
    let state = client.get_state().await?;
    println!("{} device(s), volume {}%", state.devices.len(), state.master_volume);
    client.set_volume(75).await?;
    client.set_default_output(&state.default_output.clone().unwrap()).await?;
    Ok(())
}
```

## API tour

| Area | Methods |
|---|---|
| Devices | `list_devices`, `get_device`, `rescan`, `set_default_output`, `set_default_input` |
| Volume | `volume`, `set_volume`, `device_volume`, `set_device_volume`, `mute`, `unmute` |
| Streams | `list_streams`, `set_stream_volume`, `set_stream_mute`, `move_stream`, `create_stream`, `destroy_stream` |
| Speaker groups | `list_groups`, `create_group`, `delete_group`, `add_group_member`, `remove_group_member`, `set_group_member_latency` |
| Profiles | `list_profiles`, `set_profile` |
| Effects | `effects`, `set_effects_enabled`, `set_effects_preset`, `set_equalizer_bands` |
| Microphone | `microphone`, `set_microphone_gain`, `mute_microphone`, `unmute_microphone`, `set_microphone_processing` |
| Routing / policy | `reload_routing`, `reload_policy` |
| Monitoring | `levels`, `subscribe`, `subscribe_levels` |

Master volume is the default output device's volume. `set_volume` results
carry `hw_applied: bool` — `false` means the device has no hardware volume
control (state is tracked anyway). A speaker group's id works anywhere a
device id does for stream targeting (`create_stream(app, Some(&id), None)`,
`move_stream`). Effects and equalizer bands are
per-device (an `Option<&str>` device argument, defaulting to the default
output) — see the daemon's `docs/audio-model.md` for the shapes.

## The GUI pattern (Resync)

```rust
let client = Arc::new(AudioClient::connect(SOCK).await?);

// One-shot model build.
let state = client.get_state().await?;
build_ui(&state);

// Live updates, forever, across daemon restarts.
let mut events = client.subscribe();
loop {
    match events.recv().await {
        Some(ClientEvent::Resync) => {
            // (re)connected — refetch!
            let s = client.get_state().await?;
            rebuild_ui(&s);
        }
        Some(ClientEvent::VolumeChanged { volume, .. }) => volume_slider.set(volume),
        Some(ClientEvent::DeviceAdded { id, name }) => device_list.add(id, name),
        Some(ClientEvent::DefaultOutputChanged { id }) => osd.notify_output(id),
        Some(ClientEvent::MuteChanged { muted }) => osd.show_mute(muted),
        Some(ClientEvent::EffectsChanged { device, preset, .. }) => osd.notify_preset(device, preset),
        Some(ClientEvent::GroupChanged { id }) => group_list.refresh(id),
        Some(_) => {}
        None => break, // client dropped
    }
}
```

Why this works:

- `subscribe()` never fails to *start* — the monitor connects and
  reconnects with exponential backoff (250 ms → 8 s) in the background.
- `ClientEvent::Resync` fires after every (re)connect — your single cue
  to refetch state, so you never render stale data after a daemon restart.
- `AudioClient` is `Send + Sync`; share one `Arc<AudioClient>` app-wide.
  Requests auto-reconnect with one transparent retry.
- Unknown events from newer daemons arrive as `ClientEvent::Unknown` —
  ignore and move on (protocol compatibility rule).

## Errors

`ClientError::Io` (socket trouble), `::Disconnected`, `::Protocol`
(malformed data), `::Serialization`, and `::Daemon { code, message }` —
stable codes like `device-not-found`, `invalid-volume`,
`profile-not-supported`, `permission-denied`. See the daemon's
`docs/ipc.md` for the full list.

## Run the examples

```sh
cargo run -p libmitos-audio --example volume_watch
MITOS_AUDIO_SOCK=/tmp/mitos-dev/audio.sock cargo run -p libmitos-audio --example volume_watch

cargo run -p libmitos-audio --example sine   # plays a tone through a real stream
```
