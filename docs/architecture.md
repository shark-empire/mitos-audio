# mitos-audio architecture

mitos-audio is a **system audio management service**, not an audio
driver. Linux already provides hardware → kernel → ALSA; mitos-audio is
the user-facing layer above that: device/output/input management,
per-application volume, routing, profiles, effects, and (for the pieces
that need real-time audio, not just control) a small mixing engine —
exposed to GUIs and applications over a JSON-lines IPC socket.

```
                 Linux Kernel
                      |
                     ALSA
                      |
              +-------+--------+
              |                |
        Audio hardware      Bluetooth (via BlueZ; see bluetooth/)
              |                |
              +-------+--------+
                      v
                Audio backend (backend/: alsa.rs real hardware, demo.rs no hardware needed)
                      |
                 mitos-audio (daemon/, manager/)
                      |
      +---------------+----------------+
      v               v                v
   Devices         Streams          Routing
 (devices/)       (streams/,       (routing/)
                   engine/)
      |               |                |
      +---------------+----------------+
                      v
                Control-plane IPC (ipc/) — JSON lines, one socket
                Data-plane IPC (engine/dataserver.rs) — binary frames, a second socket
                      |
      +---------------+----------------+
      v               v                v
mitos-settings    mitos-gui       Applications (via libmitos-audio, the client/ crate)
```

## Two planes, one daemon

Most audio daemons conflate "tell me what devices exist and set the
volume" with "carry the actual samples." mitos-audio keeps these
separate, on two sockets:

- **Control plane** (`ipc/`, default `/run/mitos/audio.sock`): JSON lines,
  request/response plus subscribable events. Every command in this doc's
  companion, `docs/ipc.md`, lives here. This is what `mitos-audioctl` and
  `libmitos-audio` (the `client/` crate) talk to almost all the time.
- **Data plane** (`engine/dataserver.rs`, default
  `/run/mitos/audio-data.sock`): a small binary framing protocol
  (`engine/protocol.rs`) that actually carries PCM samples from an
  application to a mixer. See `docs/audio-plane.md` for the wire format
  and its current limits (playback only — see below).

An application registers a stream once on the control plane
(`CreateStream`), then opens the data plane to actually push audio;
`engine::dataserver` and `engine::sink` handle ingest, format conversion
(`engine/convert.rs`), and mixing.

## Module map

| Module | Owns |
|---|---|
| `daemon/` | Process lifecycle, the event loop, udev hotplug watching |
| `manager/` | `AudioManager` — the central state machine every command goes through (`manager/state.rs`) |
| `backend/` | Hardware abstraction: `alsa.rs` (real ALSA), `demo.rs` (synthetic devices, no hardware needed — see below) |
| `devices/` | The `Device` model: kind, direction, bus, state, capabilities |
| `streams/`, `engine/` | The application-stream model and the actual mixing engine (ring buffers, format conversion, per-device mixer threads) |
| `volume/` | Volume/mute primitives |
| `routing/` | `routing.toml`-driven rules: device-added/removed, stream-added, profile-changed triggers → actions (see `docs/routing.md`) |
| `profiles/` | The profile catalog (`stereo`, `surround-5.1`, `bluetooth-headset`, …) and hardware-capability validation |
| `effects/` | Per-device EQ, compressor, limiter, stereo widener — see "Effects" below |
| `microphone/` | Mic DSP algorithms: AGC, noise gate, echo canceller — see "Microphone processing" below |
| `bluetooth/` | Codec negotiation and profile mapping — coordination, not a Bluetooth stack; see its own module docs |
| `groups/` | Speaker groups — synchronized multi-speaker output as a virtual device, plus the latency-compensation math (the real-time side lives in `engine/sink.rs`) |
| `policy/` | `policy.toml` — per-application permission grants (microphone access today) |
| `logging/` | Structured audit-trail logging for security-relevant events |
| `persistence/` | `state.json` — defaults, per-device volume/mute/effects, mic settings, routing memory, restored on startup |
| `ipc/` | Control-plane protocol: message shapes, permission checks (`SO_PEERCRED`) |
| `monitoring/` | Level/peak/clipping metering |
| `config/` | Loads `audio.toml`, `routing.toml`, `policy.toml` |
| `bin/mitos-audioctl.rs` | The CLI |
| `client/` (separate crate, `libmitos-audio`) | A typed async client — what an application actually links against, instead of speaking raw JSON lines itself |

## Why a demo backend

`backend::demo` provides a handful of synthetic devices (speakers,
headphones, HDMI, a Bluetooth headset, a microphone) that behave like
real hardware to everything above the backend trait — including the
mixing engine, which writes real audio into an in-memory sink instead of
ALSA. Set `backend = "demo"` in `audio.toml` (or run with no ALSA
hardware available and `backend = "auto"`) to develop or test the entire
control plane, routing engine, effects chain, and CLI without a sound
card. It's also what this project's own test suite runs against.

## Effects: wired into live audio

Unlike most of the newer modules, `effects::EffectsChain` is not just a
control-plane concept — `engine::sink`'s mixer applies a device's chain
to every mixed period before it reaches hardware (equalizer →
compressor → stereo widener → limiter, in that order; the limiter always
runs last, whenever effects are enabled, as a safety net against EQ
boosts pushing a sample over 0 dBFS). See `src/effects/mod.rs` for the
full signal chain and `docs/ipc.md` for the commands that control it.

## Microphone processing: algorithms first, wiring next

`microphone::MicrophoneProcessor` (AGC, noise gate, NLMS echo
cancellation) is real, tested DSP — but mitos-audio's data plane only
carries **playback** today (see `docs/audio-plane.md`'s limits). There is
no capture path (microphone → daemon → application) for this processor
to sit in yet, so the control-plane toggles (`SetMicrophoneProcessing`)
are tracked and persisted, but don't yet touch live audio. This is a
scope boundary, not a bug: building a symmetric capture path (mirroring
`engine::sink`'s already-working playback path) is the natural next
increment, and `MicrophoneProcessor::process` is already the exact shape
it would call.

## Bluetooth: coordination, not a stack

mitos-audio does not speak A2DP/HFP or manage pairing — a real Bluetooth
device, once connected at the OS level (BlueZ + bluealsa or similar),
already shows up as an ordinary `Device` with `bus: Bluetooth`. The
`bluetooth/` module is the audio-quality policy a future dedicated
`mitos-bluetooth` service will need: which codec to prefer
(`bluetooth::codec::negotiate`) and which mitos-audio profile a
connected profile maps to (`bluetooth::profile`). See the module's own
docs for the full rationale.

## Multi-speaker: independent vs. synchronized

Two different things people mean by "multiple speakers", handled two
different ways:

**Independent / asynchronous** — different applications on different
speakers (music on A, browser on B, game on headphones). This needed no
new machinery: a stream already targets exactly one device, every device
already gets its own sink and mixer thread, and streams on different
devices share no clock or playback position. `CreateStream` /
`MoveStream` / `routing.toml` already do this.

**Synchronized** — one stream through several speakers at once. This is
a **speaker group** (`groups/`): a virtual device with its own id that a
stream targets exactly like `"speakers"` or `"hdmi0"`, so `CreateStream`,
`MoveStream`, and the data plane's `device` field all accept a group id
with no change to how streams work. The difference is entirely in the
sink layer (`engine::sink`):

```
stream -> LiveStream (device = "group-1")
              |
      group_mixer_loop  (one mix per period, via the same mix_period()
              |          every normal sink uses, then the group's effects chain)
              |
   +----------+----------+
   v          v          v
 delay A    delay B    delay C     <- per-member DelayLine (latency compensation)
   |          |          |
 speaker A  speaker B  speaker C
```

Mixing **once** and handing the identical buffer to every member is what
keeps them sample-aligned at the source — the alternative (each member's
own mixer thread pulling from a shared stream) would have the members
*compete* for the stream's samples, since a `LiveStream`'s ring has
exactly one consumer. Each member's `DelayLine` then holds back
lower-latency members by the difference to the slowest one
(`groups::compute_delays_ms`), the same "delay the faster speaker" idea
as any multi-room audio system.

A group keeps playing on whichever members are connected: unplug a
speaker and within ~0.5 s the group sink reopens with the remaining
members; plug it back and it rejoins. Group definitions persist across
restarts even when a member is absent at boot.

What this deliberately does **not** do: measure latency for you, or
continuously correct clock drift. Latencies are configured per member
(`group-latency`), and independent hardware clocks that disagree by a few
ppm will slowly drift apart over long sessions — correcting that needs a
per-transport clock measurement (and for Bluetooth, transport-level
support) that ALSA does not expose generically. See `docs/audio-model.md`.

## Security model

Both sockets restrict connections by peer credentials
(`SO_PEERCRED`/`UCred`): root, the daemon's own uid, or members of the
`mitos-audio` group. On top of that, `policy.toml` grants or denies
individual applications microphone access. See `docs/security.md`.
