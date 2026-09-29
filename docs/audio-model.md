# Audio data model

Reference for the shapes behind `docs/ipc.md`'s commands. These are the
actual Rust types (`devices::device::Device`, `streams::stream::AudioStream`,
etc.); JSON field names match exactly.

## Device

```
Device
├── id               stable string id, e.g. "speakers", "bt-headset"
├── name             display name
├── description      longer display string
├── kind             Speakers | Headphones | Headset | Hdmi | Usb | Bluetooth | Microphone | Virtual | ...
├── direction        Output | Input | Both
├── bus              Internal | Usb | Hdmi | Bluetooth | Virtual
├── state            Disconnected | Available | Active | Suspended | Unavailable | Error
├── volume           0-100
├── muted            bool
├── channels         hardware channel count (used to validate profiles — see below)
├── sample_rates     supported rates
├── profiles         profile ids *this device* supports, e.g. ["stereo", "hdmi-stereo"]
├── active_profile   which of those is currently active
├── alsa             ALSA identifier (e.g. "hw:0,0"), when backed by real hardware
└── codec            active Bluetooth codec label (e.g. "LDAC"), if applicable — see bluetooth/
```

`profiles` (what a device supports) and the system-wide profile catalog
(`crate::profiles`, what mitos-audio knows about) are deliberately two
different lists — see the next section.

## Profiles

The catalog (`ListProfiles`) is the single source of truth for every
profile mitos-audio knows about, each with a nominal channel count:

| id | channels | id | channels |
|---|---|---|---|
| `stereo` | 2 | `surround-5.1` | 6 |
| `headphones` | 2 | `surround-7.1` | 8 |
| `headset` | 2 | `bluetooth-music` | 2 |
| `hdmi-stereo` | 2 | `bluetooth-headset` | 2 |
| | | `usb-dac` | 2 |

`SetProfile` validates against the **current default output device's own
`profiles` list and channel count** — not just "is this a real profile
name" — so activating `surround-5.1` while your default output is
2-channel headphones fails with `profile-not-supported` instead of
silently doing nothing useful. Routing-rule-triggered profile changes
(`set-profile` actions in `routing.toml`) go through the same check and
are skipped with a logged warning on failure, consistent with routing's
general "one bad rule never blocks the rest" behavior.

## Stream

```
AudioStream
├── id
├── application     e.g. "mitos-music", matched by routing.toml and policy.toml globs
├── kind            Playback | Recording | Capture | Monitoring
├── device          target device id
├── volume          0-100
├── muted           bool
├── follows_default bool — moves automatically if the default output changes
├── live            bool — has an open data-plane connection
└── buffered_ms / underrun_periods   data-plane health, when live
```

Streams are not persisted across a daemon restart by design —
applications re-register on reconnect.

## Effects

Per-device, not global — `GetEffects`/`SetEffectsPreset`/
`SetEqualizerBands` all take an optional `device` (default: default
output). Persisted per-device alongside volume/mute.

```
EffectsChain
├── enabled     master on/off for this device's chain
├── preset      Flat | Music | Movie | Game | Voice | Podcast | Custom
├── bands       10 gains in dB, at 31/62/125/250/500/1k/2k/4k/8k/16k Hz
└── width       stereo widener amount (1.0 = neutral)
```

Signal order when enabled: equalizer → compressor (if the preset turns
one on) → stereo widener → limiter (always, last). See
`docs/architecture.md`'s "Effects" section for why the limiter is
unconditional.

## Speaker groups

```
SpeakerGroup
├── id         "group-N", assigned at creation
├── name       display name
└── members[]
    ├── device_id    an existing output device
    └── latency_ms   configured output latency of that device (0 = none requested)
```

A group id is accepted wherever a stream target is: `CreateStream`'s
`device`, `MoveStream`'s `device_id`, the data plane's OPEN `device`.
It is **not** accepted as the system default output (`SetDefaultOutput`)
— profiles, master volume and hardware mute all assume the default
output is a real device with channels and a mixer.

Every member receives the identical mixed buffer. To line members up,
each is delayed by `max(latency_ms) - its own latency_ms`, so the
slowest member sets the pace:

| member | configured latency | delay applied |
|---|---|---|
| Speaker A | 20 ms | 60 ms |
| Speaker B | 80 ms | 0 ms |

**What is and isn't automatic.** `latency_ms` is something you set
(`mitos-audioctl group-latency`); mitos-audio does not measure acoustic
or network latency, because that needs a per-transport round trip this
project has no input for — Bluetooth in particular. Continuous
clock-drift correction (two independent hardware clocks nominally at
48 kHz disagreeing by a few ppm) is likewise not implemented: over a
long session two members can slowly walk apart. The group is one mix,
delivered at the same instant, with configured offsets — accurate to
hardware buffer granularity for local devices, best-effort beyond that.

Effects are applied once per group, before fan-out. A group has its own
effects chain, addressed by its id like any device
(`mitos-audioctl effects-preset music --device group-1`) and persisted
with the group. A member's *own* per-device effects chain is **not**
applied while it plays as part of a group — the group's mixer writes to
the member directly — so tune the group's chain, not the members'.

**A device that is both a group member and a direct stream target at the
same time** gets opened twice (once by its own sink, once by the group's
fan-out) — whether that works depends on the backend supporting
concurrent opens of the same hardware (ALSA's `dmix`, where configured,
usually does; a bare exclusive `hw:` device will not). Routing the same
device both ways isn't recommended until this is worth a proper shared
device-handle cache; using a device only inside the group (or only
directly) sidesteps it entirely.

## Microphone

```
GetMicrophone
├── device               current default input, if any
├── muted
├── gain_db              -30..30
├── noise_suppression    bool
├── echo_cancellation    bool
└── agc                  bool
```

The last three are control-plane state only until the capture data plane
exists — see `docs/architecture.md`.
