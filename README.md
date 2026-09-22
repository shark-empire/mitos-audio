# mitos-audio
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