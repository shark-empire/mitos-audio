# mitos-audio
Audio plane (mix, route, meter) ✓ under working; move "native playback lib" out of stubbed. Quick demo gains mitos-audioctl tone.

15. Try it

bash

cargo build && cargo test -p mitos-audio   # includes convert tests

# Dev, demo backend — full audio plane without hardware:
mkdir -p /tmp/mitos-dev
printf 'socket_path = "/tmp/mitos-dev/audio.sock"\nstate_path = "/tmp/mitos-dev/state.json"\nrouting_path = "/tmp/mitos-dev/routing.toml"\nbackend = "demo"\n' > /tmp/mitos-dev/audio.toml
MITOS_AUDIO_CONFIG=/tmp/mitos-dev/audio.toml ./target/debug/mitos-audio &

CTL="--socket /tmp/mitos-dev/audio.sock"
./target/debug/mitos-audioctl $CTL tone                 # sound + a real stream
./target/debug/mitos-audioctl $CTL streams              # live: true, buffered_ms
./target/debug/mitos-audioctl $CTL watch                # OUT meter dances with the tone
./target/debug/mitos-audioctl $CTL new-stream "My Game" # routing-test placeholder

# Real hardware: same commands — tone comes out of your speakers.
# Per-app mixing:
./target/debug/mitos-audioctl $CTL tone --freq 300 &    # stream A
./target/debug/mitos-audioctl $CTL tone --freq 500 &    # stream B — hear both mixed
While tones play: mitos-audioctl streams shows both, and SetStreamVolume on one (mitos-audioctl streams → future CLI sugar) — or via libmitos-audio — changes its loudness within ~12 ms. Plug a USB headset mid-tone: udev → DeviceAdded → routing rule moves the stream → the tone jumps devices seamlessly.

Honest caveats

 hw: vs plughw:: sinks open plughw:X,Y — if another application holds the raw device exclusively, open retries every 2 s (debug-logged).
 MSRV/versions: group permissions want Rust ≥ 1.73 + tokio ≥ 1.24; both degrade gracefully.
 The v0.3 S16_LE typo: alsa-rs spells it S16LE — fixed above; patch it if you copied v0.3's capture-meter verbatim.
 Streams are still not persisted (correct — apps re-register), and capture has no data path yet.

| Levels | subscribe_levels (10 Hz frames, auto park when dropped), levels || Streams | create_stream (app registration, fires routing rules), destroy_stream || Routing | reload_routing |

13. Try it (real hardware)

bash

cargo build   # needs libasound2-dev; udev needs no extra libs

mkdir -p /tmp/mitos-dev
printf 'socket_path = "/tmp/mitos-dev/audio.sock"\nstate_path = "/tmp/mitos-dev/state.json"\nrouting_path = "/tmp/mitos-dev/routing.toml"\n' > /tmp/mitos-dev/audio.toml
MITOS_AUDIO_CONFIG=/tmp/mitos-dev/audio.toml ./target/debug/mitos-audio
# → "audio backend selected: alsa", "udev hotplug watcher active", "routing rules loaded"

./target/debug/mitos-audioctl --socket /tmp/mitos-dev/audio.sock watch   # talk → see your mic!
# plug a USB headset → within ~400 ms: DeviceAdded + rule fires (default → headset)
# unplug → DeviceRemoved + restore-output (back to speakers)
# change volume in alsamixer → within 3 s: VolumeChanged + DeviceChanged
# edit /tmp/mitos-dev/routing.toml → mitos-audioctl --socket … reload-routing
Notes & honest caveats

 udev crate API: code targets udev = "0.9"; on older versions socket.iter(None) becomes socket.iter() — one-line fix if your lockfile resolves differently.
 Output metering is 0.0 on real ALSA by design: a management service above ALSA can't tap output samples without being the mixer. Input metering works today (capture PCM); output meters come alive when the roadmap's audio plane routes samples through mitos-audio.
 Capture metering opens default: shareable via dsnoop on standard distros; if a device grabs the mic exclusively, metering holds its last value and retries (1 s backoff). metering.capture_input = false turns it off entirely.
 Streams are still transient (not persisted across restarts) — correct behavior; apps re-register.