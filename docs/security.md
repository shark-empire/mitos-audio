# Security model

## Connection permissions

Both the control-plane socket (`/run/mitos/audio.sock`) and the
data-plane socket (`/run/mitos/audio-data.sock`) check the connecting
peer's credentials (`SO_PEERCRED`, exposed by tokio as `UCred`) before
accepting anything from it — see `ipc/permissions.rs`. A connection is
accepted if the peer is:

- root (uid 0),
- the daemon's own uid (useful for the daemon's own tools), or
- a member of the `mitos-audio` group.

Anything else is rejected and logged (see "Audit log" below) — the
connection is closed without reading a single command from it.

Set this up once, on install:

```sh
sudo groupadd -r mitos-audio
sudo usermod -aG mitos-audio <user or app service account>
```

`services/mitos-audio.service` also grants the daemon itself
`SupplementaryGroups=audio` — that's the *system* `audio` group that owns
`/dev/snd/*` device nodes, unrelated to the `mitos-audio` group above,
which is about who may **talk to the daemon**, not who may touch ALSA
hardware directly.

## Application policy (`policy.toml`)

On top of "may this peer connect at all," `policy.toml` grants or denies
individual **applications** access to specific things — today, the
microphone. It's checked when an application registers a recording or
capture stream (`CreateStream`):

```toml
default_microphone = "allow"

[[app]]
name = "mitos-*"          # glob, case-insensitive
microphone = "allow"

[[app]]
name = "sketchy-widget"
microphone = "deny"
```

First matching `[[app]]` rule wins (file order); with no match,
`default_microphone` applies. A missing or invalid policy file allows
every application — a policy is an opt-in restriction, not a
requirement — logged at `warn` so that's never a silent surprise.
Reload after editing without restarting the daemon:

```sh
mitos-audioctl reload-policy
```

A denied request gets `permission-denied` back over IPC and never
reaches the point of creating a stream.

## Audit log

Security-relevant events are logged through `logging::audit` on a
dedicated `tracing` target (`audit`) rather than a bespoke log file, so
any standard `tracing-subscriber`/journald setup can select or route them
without mitos-audio reinventing log shipping:

- connection rejected (control or data plane), with the peer's uid
- microphone access granted/denied (`policy.toml`)
- policy or routing rules reloaded

To see only these, filter on the target — for example, with the
`tracing-subscriber` `EnvFilter` already in use:

```sh
RUST_LOG=mitos_audio::audit=debug,warn mitos-audio
```

or with journald, filter on the daemon's unit and grep for these
messages; the `target: "audit"` field is present in the structured
fields tracing-subscriber emits even in plain-text mode.

## What's intentionally out of scope here

- **Encryption**: both sockets are local Unix domain sockets with
  filesystem permissions and peer-credential checks; there's no
  network exposure to encrypt against.
- **Per-application volume/routing permissions**: `policy.toml` covers
  microphone access because that's the sensitive one; volume and
  routing control aren't gated by application identity today.
- **Bluetooth pairing security**: entirely BlueZ's responsibility — see
  `docs/architecture.md`'s "Bluetooth" section.
