# Troubleshooting

## No sound / "no such device" errors

1. Check what the daemon actually sees:
   ```sh
   mitos-audioctl devices
   ```
   If a device you expect isn't listed, the ALSA backend never scanned it
   — check `dmesg`/`aplay -l` to confirm the kernel sees the hardware at
   all before looking further at mitos-audio.
2. Check which backend is active:
   ```sh
   mitos-audioctl ping   # or: GetState via socat, see docs/ipc.md
   ```
   `GetState`'s `backend` field is `"alsa"` or `"demo"`. If it says
   `"demo"` and you expected real hardware, `audio.toml`'s `backend` is
   probably set to `"demo"` explicitly, or `"auto"` fell back because
   opening ALSA failed — check the daemon's logs for why.

## "device or resource busy"

Another process (PulseAudio, PipeWire, another instance of mitos-audio)
already holds the ALSA device exclusively. mitos-audio opens via
`plughw:` with a fallback/retry, not `hw:` directly (see the "Honest
caveats" in the top-level README) — if this persists, check
`fuser /dev/snd/*` or `lsof` for the other holder.

## "connection rejected" / can't connect to the socket at all

This is the permission model in `docs/security.md`: the connecting
process must be root, the daemon's own uid, or in the `mitos-audio`
group. Check group membership:

```sh
groups <user>
```

and that you started a new session (or `newgrp mitos-audio`) after being
added — group membership changes don't apply retroactively to already-running
processes.

## "permission-denied" specifically on a microphone/recording stream

That's `policy.toml`, not the connection-level check above — see
`docs/security.md`'s "Application policy" section. Check what's actually
configured:

```sh
cat /etc/mitos/policy.toml
```

and reload after fixing it with `mitos-audioctl reload-policy` rather
than restarting the daemon.

## Profile activation fails with `profile-not-supported`

The profile you asked for is real, but the **current default output
device** doesn't declare support for it (or doesn't have enough
channels) — see `docs/audio-model.md`'s profiles table. Check what a
device actually supports:

```sh
mitos-audioctl devices    # channel counts
mitos-audioctl profiles   # full catalog with per-profile channel counts
```

Switching to a device that does support the profile first (or picking a
profile the current device lists) resolves it.

## Effects sound wrong / distorted

- Confirm effects are actually enabled for the device you're listening
  through (`mitos-audioctl effects --device <id>`) — presets are stored
  per device, and `SetEffectsPreset`/`SetEqualizerBands` silently target
  whatever device you pass (or the default output).
- Extreme custom EQ boosts (`mitos-audioctl eq ...`) plus a loud source
  can drive the compressor and limiter hard; that's the limiter doing
  its job (preventing clipping), not a bug, but very large boosts will
  still sound "squashed." Start from a preset and adjust from there.

## Microphone DSP toggles don't seem to do anything

By design, for now — see `docs/architecture.md`'s "Microphone
processing" section. `SetMicrophoneProcessing` toggles are real,
persisted state, but there is no capture data path yet for the DSP to
run on. This isn't backend- or hardware-specific; it's the same on ALSA
and demo.

## Where to look next

- `docs/architecture.md` — how the pieces fit together
- `docs/ipc.md` — every command/event/error code
- `docs/audio-plane.md` — the data-plane wire format and its current
  limits (playback only, no capture, linear resampling)
- `docs/routing.md` — `routing.toml` rule syntax
- `docs/security.md` — permissions and the audit log
