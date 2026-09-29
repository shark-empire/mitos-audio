# mitos-audio IPC protocol reference

## Transport & framing

- Two Unix domain sockets: control plane (default `/run/mitos/audio.sock`)
  and data plane (default `/run/mitos/audio-data.sock`, binary framing —
  see `docs/audio-plane.md`). This document covers the control plane.
- UTF-8 JSON, one message per line (`\n`-terminated). Max line length: 1 MiB.
- Both sockets restrict connections to root, the daemon's own uid, or
  members of the `mitos-audio` group — see `docs/security.md`.

## Message shapes

Request (client → daemon):
```json
{ "id": 1, "command": "SetVolume", "params": { "volume": 60 } }
```

Response (daemon → client):
```json
{ "id": 1, "ok": true, "result": { "volume": 60 } }
{ "id": 2, "ok": false, "error": { "code": "device-not-found", "message": "device not found: foo" } }
```

Event (daemon → subscribed clients):
```json
{ "event": "VolumeChanged", "data": { "device": null, "volume": 60 } }
```

Responses are sent in request order per connection. On a subscribed
connection, events interleave with responses — distinguish by the
presence of `"id"` (response) vs `"event"` (event). Clients must ignore
unknown fields and unknown event names: the protocol is additive-only
within a minor series, so a newer daemon talking to an older client
should never break it.

## Commands

| Command | Params | Result |
|---|---|---|
| `Ping` | – | `{ pong: true }` |
| `GetState` | – | full snapshot: devices, streams, groups, defaults, volumes, mic, profile, effects, backend, version |
| `GetDefaults` | – | `{ default_output, default_input }` |
| `ListDevices` | – | `{ devices: [...], default_output, default_input }` |
| `GetDevice` | `id` | device object (see `docs/audio-model.md`) |
| `Rescan` | – | `{ devices: [...], default_output, default_input }` — forces a hardware rescan (also automatic every ~3 s) |
| `SetDefaultOutput` | `id` | `{ default_output }` |
| `SetDefaultInput` | `id` | `{ default_input }` |
| `GetVolume` | `device?` | `{ device, volume, muted }` — resolves to the default output when `device` is omitted |
| `SetVolume` | `volume`, `device?` | `{ volume, hw_applied }` — `hw_applied` is `false` when the device has no hardware volume control (state is still tracked) |
| `Mute` / `Unmute` | – | `{ muted, hw_applied }` |
| `ListStreams` | – | `{ streams: [...] }` |
| `SetStreamVolume` | `stream_id`, `volume` | `{ stream_id, volume }` |
| `SetStreamMute` | `stream_id`, `mute` | `{ stream_id, muted }` |
| `MoveStream` | `stream_id`, `device_id` (a device or speaker-group id) | `{ stream_id, device }` |
| `CreateStream` | `application`, `device?` (a device **or speaker-group** id), `kind?` (`playback`\|`recording`\|`capture`\|`monitoring`) | `{ stream }` — interim application stream registration; fires `stream-added` routing rules; a `recording`/`capture` kind is checked against `policy.toml` first (see `docs/security.md`) |
| `DestroyStream` | `stream_id` | `{ removed }` |
| `ListProfiles` | – | `{ profiles: [...], active, catalog: [{id, label, channels, description}, ...] }` |
| `SetProfile` | `profile` | `{ profile }` — validated against the current default output's own capabilities, not just the name; see `docs/audio-model.md` |
| `GetMicrophone` | – | `{ device, muted, gain_db, noise_suppression, echo_cancellation, agc }` |
| `SetMicrophoneGain` | `gain` (dB, clamped ±30) | `{ gain_db }` |
| `MuteMicrophone` | `mute` | `{ muted }` |
| `SetMicrophoneProcessing` | `noise_suppression?`, `echo_cancellation?`, `agc?` (each `bool`, omit to leave unchanged) | `{ noise_suppression, echo_cancellation, agc }` — control-plane state; see `docs/architecture.md` for why this doesn't touch live audio yet |
| `GetEffects` | `device?` (device or group id) | `{ device, enabled, preset, bands: [10 floats], width }` |
| `SetEffectsEnabled` | `device?`, `enabled` | `{ device, enabled }` |
| `SetEffectsPreset` | `device?`, `preset` (`flat`\|`music`\|`movie`\|`game`\|`voice`\|`podcast`) | `{ device, preset }` |
| `SetEqualizerBands` | `device?`, `bands` (exactly 10 dB values) | `{ device, bands, preset: "custom" }` |
| `ListGroups` | – | `{ groups: [{ id, name, members: [{ device_id, latency_ms }] }] }` |
| `CreateGroup` | `name`, `members` (device ids) | `{ id, name, members }` — a group is a virtual output; see `docs/audio-model.md` |
| `DeleteGroup` | `id` | `{ removed }` |
| `AddGroupMember` | `id`, `device_id`, `latency_ms?` | `{ id, members }` (count) |
| `RemoveGroupMember` | `id`, `device_id` | `{ id, members }` (count) |
| `SetGroupMemberLatency` | `id`, `device_id`, `latency_ms` | `{ id, device_id, latency_ms }` — faster members are delayed to match the slowest |
| `GetLevels` | – | `{ master_volume, muted, output_level, input_level, peak, clipping }` |
| `SubscribeEvents` | – | `{ subscribed: true }` then event lines |
| `SubscribeLevels` | – | `{ subscribed: true }`, then `LevelChanged` event lines at ~`interval_ms` — a **dedicated** subscription, separate from `SubscribeEvents`; pushed only while ≥1 level subscriber exists |
| `ReloadRouting` | – | `{ rules: n }` — re-reads `routing.toml` |
| `ReloadPolicy` | – | `{ rules: n }` — re-reads `policy.toml` |

## Events

| Event | Data | Emitted when |
|---|---|---|
| `DeviceAdded` | `{ id, name }` | device hotplug |
| `DeviceRemoved` | `{ id }` | device removal |
| `DeviceChanged` | `{ id }` | state/profile change |
| `DefaultOutputChanged` | `{ id }` | default output switched |
| `DefaultInputChanged` | `{ id }` | default input switched |
| `VolumeChanged` | `{ device?, volume }` | master or per-device volume change |
| `MuteChanged` | `{ muted }` | master mute toggled |
| `StreamAdded` | `{ id, application }` | application opens a stream |
| `StreamRemoved` | `{ id }` | application closes a stream |
| `StreamChanged` | `{ id }` | stream volume/mute/device changed |
| `ProfileChanged` | `{ profile }` | profile activated |
| `MicrophoneChanged` | `{ muted, gain, noise_suppression, echo_cancellation, agc }` | any microphone setting changed — always the full snapshot, not just what you set |
| `EffectsChanged` | `{ device, enabled, preset, bands }` | any effects command changed a device's chain |
| `GroupChanged` | `{ id }` | a group was created, or its membership/latency changed |
| `GroupRemoved` | `{ id }` | a group was deleted |
| `LevelChanged` | `{ output_level, input_level, peak, clipping }` | ~10 Hz, `SubscribeLevels` connections only — never sent on `SubscribeEvents` |

## Error codes

`device-not-found`, `stream-not-found`, `invalid-volume`,
`invalid-profile`, `profile-not-supported`, `permission-denied`,
`not-an-output`, `not-an-input`, `ipc-error`, `config-error`,
`config-parse-error`, `io-error`, `serialization-error`

New codes are additive — treat any code you don't recognize as a generic
failure rather than erroring on the parse itself.

## Semantics notes

- **Persistence**: defaults, per-device volume/mute/effects, mic
  settings, and routing memory are restored on daemon start from
  `state_path` (atomic write, 400 ms debounce). Disable with
  `persist = false`. Streams themselves are not persisted — applications
  re-register on reconnect.
- **Master volume** = the default output device's volume. `GetVolume` /
  `SetVolume` / `Mute` / `Unmute` without a `device` resolve to the
  current default output; results and `VolumeChanged` events carry its id.
- **Routing**: `routing.toml` rules fire on device-added / device-removed
  / stream-added / profile-changed — see `docs/routing.md`.
  `ReloadRouting` re-reads the file at runtime without restarting.
- **Policy**: `policy.toml` grants/denies per-application microphone
  access, checked on `CreateStream` — see `docs/security.md`.
  `ReloadPolicy` re-reads it at runtime.
- **Metering**: `SubscribeLevels` is a separate subscription from
  `SubscribeEvents` and receives only `LevelChanged` frames (~10 Hz),
  pushed only while at least one level subscriber exists — idle GUIs cost
  nothing, and ALSA capture metering parks its PCM when the last meter
  closes. `GetLevels` returns the last measured frame (may be stale with
  no subscribers). Levels reflect audio **after** the effects chain, so
  clipping detection matches what actually reached hardware.
- **Speaker groups**: a group id is accepted anywhere a stream target
  is (`CreateStream`, `MoveStream`, the data plane's OPEN `device`) and
  by the effects commands, but not by `SetDefaultOutput`. Membership or
  latency changes take effect within ~0.5 s (the group sink reopens);
  unplugging a member does the same automatically. See
  `docs/architecture.md`'s "Multi-speaker" section.
- **Effects**: applied live, per device, in the mixer — see
  `docs/architecture.md`. Persisted per-device.
- **Microphone DSP**: `SetMicrophoneProcessing` toggles are real,
  persisted control-plane state; they don't affect live audio yet because
  there is no capture data plane — see `docs/architecture.md`.
- **Hotplug**: a udev watcher (subsystem `sound`) triggers a debounced
  instant rescan, plus a periodic poll (default 3 s) that also catches
  external mixer changes (e.g. `alsamixer`).
- **Backend**: `GetState` includes `"backend": "alsa"` or `"demo"`.

## Testing without writing a client

```sh
socat - UNIX-CONNECT:/run/mitos/audio.sock
{"id":1,"command":"ListDevices","params":{}}
```

or use the CLI's raw event tap: `mitos-audioctl monitor | jq .`
