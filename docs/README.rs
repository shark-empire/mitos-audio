mitos-audio

System audio management service for MITOS. Sits above ALSA — it is nota driver — and exposes one IPC API to mitos-gui, mitos-settings,applications, and the CLI.

Status (v0.1 — foundation)

Working now	Stubbed / next
JSON-lines IPC daemon (Unix socket)	ALSA backend (demo devices seeded)
Device / stream / volume state model	udev hotplug detection
22 commands + 12 event types	Routing rules engine
Event broadcast (pub/sub)	Persistence (database/settings)
mitos-audioctl CLI	Bluetooth (via mitos-bluetooth)
Config, permissions, systemd unit	Effects, mic processing, metering
Build & run

cargo build# Dev run (no root needed):mkdir -p /tmp/mitos-devecho 'socket_path = "/tmp/mitos-dev/audio.sock"' > /tmp/mitos-dev/audio.tomlMITOS_AUDIO_CONFIG=/tmp/mitos-dev/audio.toml ./target/debug/mitos-audio# In another terminal:./target/debug/mitos-audioctl --socket /tmp/mitos-dev/audio.sock devices./target/debug/mitos-audioctl --socket /tmp/mitos-dev/audio.sock volume 80./target/debug/mitos-audioctl --socket /tmp/mitos-dev/audio.sock monitor
Documentation

docs/ipc.md — full protocol reference
docs/integration.md — connecting mitos-gui, mitos-settings, applications
5. Try it right now

bash

cargo build

# Terminal 1 — daemon (dev config, no root):
mkdir -p /tmp/mitos-dev
echo 'socket_path = "/tmp/mitos-dev/audio.sock"' > /tmp/mitos-dev/audio.toml
MITOS_AUDIO_CONFIG=/tmp/mitos-dev/audio.toml ./target/debug/mitos-audio

# Terminal 2 — use it:
./target/debug/mitos-audioctl --socket /tmp/mitos-dev/audio.sock devices
./target/debug/mitos-audioctl --socket /tmp/mitos-dev/audio.sock volume 80
./target/debug/mitos-audioctl --socket /tmp/mitos-dev/audio.sock output headphones
./target/debug/mitos-audioctl --socket /tmp/mitos-dev/audio.sock streams
Run monitor in a third terminal while changing the volume in terminal 2 — you'll see the live VolumeChanged / DefaultOutputChanged events, exactly what mitos-gui will consume.

6. Next steps (in roadmap order)

src/backend/alsa.rs — real device enumeration via the alsa crate, replacing the demo seed
udev hotplug → DeviceAdded/DeviceRemoved events for real
Persistence — survive restarts (defaults, volumes, profiles)
Routing rules engine — the Bluetooth connect/disconnect auto-switching from the roadmap
libmitos-audio client crate — typed Rust client so mitos-gui/mitos-settings don't hand-roll sockets
Metering events — LevelChanged at ~10 Hz for live GUI meters
Bluetooth coordination with your future mitos-bluetooth service