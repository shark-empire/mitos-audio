# mitos-audio

A system audio management service for MITOS — the user-facing layer
**above** ALSA, not a driver:

```
hardware → kernel → ALSA → mitos-audio → mitos-settings / mitos-gui / applications
```

mitos-audio owns device and output/input management, per-application
volume, hotplug-aware routing, audio profiles, per-device effects, and a
small real-time mixing engine — all exposed over a JSON-lines IPC socket,
with a typed async Rust client (`libmitos-audio`, the `client/` crate) so
applications don't have to speak the wire protocol by hand.

## Features

- **Device management** — internal speakers, headphones, headsets, USB
  audio, HDMI/DisplayPort, Bluetooth, USB DACs, microphones, virtual
  devices. Hotplug via udev with a periodic-poll safety net.
- **Per-application volume** — every stream gets its own volume, mute,
  and target device, independent of the system volume.
- **Routing** — `routing.toml` rules react to device hotplug, new
  streams, and profile changes (e.g. "Bluetooth headset connected →
  switch default output → restore previous output on disconnect"). See
  `docs/routing.md`.
- **Profiles** — stereo, surround 5.1/7.1, HDMI, Bluetooth music/headset,
  USB DAC, etc., validated against what the active device actually
  supports (not just a name check) — see `docs/audio-model.md`.
- **Effects** — a real, live equalizer/compressor/stereo-widener/limiter
  chain per device, with named presets (Flat/Music/Movie/Game/Voice/
  Podcast) or custom 10-band EQ. Applied in the mixer, not just tracked
  as settings.
- **Microphone** — mute, gain, and (as of this release) automatic gain
  control, noise suppression, and echo cancellation algorithms, unit
  tested against synthetic signals. See "Current limits" below for what
  this does and doesn't do to live audio yet.
- **Bluetooth coordination** — codec preference negotiation and profile
  mapping for a future `mitos-bluetooth` service, without mitos-audio
  duplicating BlueZ's job.
- **Security** — peer-credential connection checks (root / daemon uid /
  `mitos-audio` group) plus `policy.toml` for per-application microphone
  grants, with an audit-log trail. See `docs/security.md`.
- **Persistence** — defaults, per-device volume/mute/effects, mic
  settings, and routing memory survive a restart (atomic writes, 400 ms
  debounce).

See `docs/architecture.md` for how the pieces fit together, and
`docs/ipc.md` for the full command/event reference.

## Quick start — no hardware needed

The `demo` backend provides synthetic devices (including a Bluetooth
headset with codec negotiation already run) and a fully working mixing
engine writing to an in-memory sink, so you can exercise the entire
control plane without a sound card:

```sh
cargo build && cargo test   # unit tests run entirely against pure logic / the demo backend

mkdir -p /tmp/mitos-dev
cat > /tmp/mitos-dev/audio.toml <<'EOF'
socket_path = "/tmp/mitos-dev/audio.sock"
data_socket_path = "/tmp/mitos-dev/audio-data.sock"
state_path = "/tmp/mitos-dev/state.json"
routing_path = "/tmp/mitos-dev/routing.toml"
policy_path = "/tmp/mitos-dev/policy.toml"
backend = "demo"
EOF
MITOS_AUDIO_CONFIG=/tmp/mitos-dev/audio.toml ./target/debug/mitos-audio &

CTL="--socket /tmp/mitos-dev/audio.sock"
./target/debug/mitos-audioctl $CTL tone                     # generates a real stream you can hear
./target/debug/mitos-audioctl $CTL streams                  # live: true, buffered_ms
./target/debug/mitos-audioctl $CTL watch                     # the output meter dances with the tone
./target/debug/mitos-audioctl $CTL effects-preset music      # hear the EQ/compressor kick in
./target/debug/mitos-audioctl $CTL tone --freq 300 &         # a second stream
./target/debug/mitos-audioctl $CTL tone --freq 500 &         # both mix together, audibly
```

Plug in a device (real hardware) or watch `mitos-audioctl monitor | jq .`
(demo) to see routing react to hotplug.

## Quick start — real hardware

```sh
cargo build   # needs libasound2-dev; udev hotplug needs no extra libs

mkdir -p /tmp/mitos-dev
cat > /tmp/mitos-dev/audio.toml <<'EOF'
socket_path = "/tmp/mitos-dev/audio.sock"
state_path = "/tmp/mitos-dev/state.json"
routing_path = "/tmp/mitos-dev/routing.toml"
policy_path = "/tmp/mitos-dev/policy.toml"
EOF
MITOS_AUDIO_CONFIG=/tmp/mitos-dev/audio.toml ./target/debug/mitos-audio
# → "audio backend selected: alsa", "udev hotplug watcher active", "routing rules loaded"

./target/debug/mitos-audioctl --socket /tmp/mitos-dev/audio.sock watch   # talk — see your mic level
# plug in a USB headset → within ~400 ms: DeviceAdded + a routing rule can switch the default
# unplug → DeviceRemoved + restore-previous-output
# change volume in alsamixer → within 3 s: VolumeChanged + DeviceChanged
# edit routing.toml, then: mitos-audioctl --socket ... reload-routing
```

For a real install rather than a dev sandbox, use `services/mitos-audio.service`
and the default paths under `/etc/mitos` and `/run/mitos` (see
`config/audio.toml`, `config/routing.toml`, `config/policy.toml`).

## The CLI

`mitos-audioctl --help` lists every command; the highlights:

```sh
mitos-audioctl devices                    # list all devices
mitos-audioctl output headphones          # switch default output
mitos-audioctl volume 80                  # master volume
mitos-audioctl mic-mute
mitos-audioctl profile surround-5.1       # fails cleanly if the active device can't do it
mitos-audioctl effects-preset podcast
mitos-audioctl eq -- 3 2 0 -1 -1 0 1 2 3 3   # custom 10-band EQ, in dB (`--` before negative values)
mitos-audioctl mic-dsp --noise-suppression true --agc true
mitos-audioctl reload-routing
mitos-audioctl reload-policy
mitos-audioctl monitor | jq .             # raw event stream
```

## The client library (`libmitos-audio`)

Applications should link `client/` rather than speak JSON lines by hand:

```rust
let client = mitos_audio_client::AudioClient::connect("/run/mitos/audio.sock").await?;
client.set_volume(60).await?;
let mut events = client.subscribe();
while let Some(event) = events.recv().await {
    // ClientEvent::VolumeChanged, ::EffectsChanged, ::MicrophoneChanged, ...
}
```

See `client/README.md` and `client/examples/` (`tone.rs` plays a sine wave
through a real stream; `volume_watch.rs` prints events as they arrive).

## Building & testing

```sh
cargo build --workspace
cargo test --workspace
```

Every new module added in this release (`profiles`, `policy`, `logging`,
`microphone`, `bluetooth`, `effects`) is unit tested inline
(`#[cfg(test)] mod tests` next to the code, matching this repo's existing
convention) against synthetic signals and the `demo` backend — none of it
needs real audio hardware to verify.

## Documentation

- `docs/architecture.md` — how the pieces fit together
- `docs/audio-model.md` — Device / Stream / Profile / Effects shapes
- `docs/ipc.md` — full command/event/error-code reference
- `docs/audio-plane.md` — the data-plane wire format and its limits
- `docs/routing.md` — `routing.toml` syntax
- `docs/security.md` — connection permissions, `policy.toml`, audit log
- `docs/troubleshooting.md` — common problems and how to diagnose them
- `docs/integration.md` — how mitos-gui/mitos-settings/applications integrate

## Current limits

Documented honestly rather than glossed over:

- **No capture data plane yet.** The data plane carries playback only —
  see `docs/audio-plane.md`. Microphone AGC/noise-suppression/echo-
  cancellation are real, tested algorithms with real control-plane
  toggles, but nothing feeds live captured audio through them yet.
  Building that path (mirroring the already-working playback path) is
  the natural next increment.
- **Bluetooth is coordination, not a stack.** mitos-audio negotiates
  which codec *should* be used and maps A2DP/HFP/HSP onto its own
  profiles; it does not talk to a Bluetooth controller. A real Bluetooth
  device already shows up as an ordinary device via BlueZ + bluealsa (or
  similar) today.
- **Effects are basic-but-real DSP**, not a mastering suite: the
  equalizer, compressor, and limiter are standard textbook designs
  (RBJ-cookbook biquads, a feed-forward compressor, an instant-attack
  limiter); the "spatial" stage is mid/side stereo widening, not HRTF-based
  3D audio or measured room correction.
- **`hw:` vs `plughw:`**: sinks open `plughw:X,Y`; if another process
  holds a device exclusively, opening retries every 2 s (debug-logged).
- **Streams are not persisted across restarts** — by design, applications
  re-register on reconnect.
- Peer-credential checks need a reasonably recent Rust/tokio
  (`UCred`/`SO_PEERCRED` support); both degrade gracefully if unavailable.

## License

MIT — see `LICENSE`.
