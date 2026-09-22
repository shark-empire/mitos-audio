# mitos-audio
Status (v0.2)

Working now	Stubbed / next
ALSA backend: real enumeration + HW volume	udev hotplug (3 s poll for now)
Hotplug & external changes → events	Routing rules engine
Demo backend for dev/CI (auto-fallback)	Persistence
23 commands + 12 events, master=def. output	Bluetooth (via mitos-bluetooth)
libmitos-audio client crate	Effects, mic processing, metering
mitos-audioctl CLI incl. rescan	Real application streams (audio plane)
Build

# Real ALSA backend (default) — needs libasound2-dev / alsa-lib-devel:cargo build# Demo backend only (CI, containers, non-Linux):cargo build -p mitos-audio --no-default-features# Client crate alone:cargo build -p libmitos-audio
Backend selection at runtime via /etc/mitos/audio.toml: backend = "auto" (default) | "alsa" | "demo".

text


---

# Try it

```bash
cargo build

# Terminal 1 — daemon (dev socket; on a real Linux desktop this uses ALSA):
mkdir -p /tmp/mitos-dev
echo 'socket_path = "/tmp/mitos-dev/audio.sock"' > /tmp/mitos-dev/audio.toml
MITOS_AUDIO_CONFIG=/tmp/mitos-dev/audio.toml ./target/debug/mitos-audio
# → "audio backend selected: alsa"  (or "demo" if no sound card)

# Terminal 2 — real hardware devices:
./target/debug/mitos-audioctl --socket /tmp/mitos-dev/audio.sock devices
./target/debug/mitos-audioctl --socket /tmp/mitos-dev/audio.sock volume 40   # actual mixer!

# Terminal 3 — the client crate watching events:
MITOS_AUDIO_SOCK=/tmp/mitos-dev/audio.sock \
  cargo run -p libmitos-audio --example volume_watch
Then the fun test: plug or unplug a USB audio device (or change something in alsamixer in another terminal) — within ~3 s you'll see DeviceAdded / DeviceRemoved / DeviceChanged events flow into volume_watch and mitos-audioctl monitor, exactly what mitos-gui will consume.