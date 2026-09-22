mitos-audio — Audio Plane

How applications deliver actual audio to mitos-audio, and how it is mixedand routed to devices. Companion to docs/ipc.md (control plane).

Architecture

application ──control──► /run/mitos/audio.sock        (JSONL: volumes, moves, events)application ──data────►  /run/mitos/audio-data.sock   (binary: PCM frames)data plane ingest: decode → stereo → resample → ring buffer (1 s)sink per target device: mix (sum, per-stream volume²) → clamp → ALSA
Two sockets by design: the control plane stays pure JSON-lines; the dataplane is pure binary framing. Either can be used without the other.

Wire protocol

Frame = [u32 payload_len][u8 type][payload], little-endian, max 1 MiB.

Type	Dir	Name	Payload
0x01	→	OPEN	JSON: {application, sample_rate, channels, format, device?}
0x02	→	DATA	raw PCM (interleaved, negotiated format)
0x03	→	CLOSE	empty
0x81	←	OPEN_ACK	JSON: {stream_id}
0x82	←	ERROR	JSON: {code, message}
0x83	←	CLOSED	{}
One stream per connection. OPEN creates the stream (visible instreams, fires stream-added routing rules); DATA feeds it; CLOSE (ordropping the socket) destroys it (StreamRemoved).

Formats: s16le, f32le; 1–8 channels; any rate 1 kHz–384 kHz. All inputis converted to the canonical internal format (48 kHz stereo f32):mono is duplicated, >2 channels take the first two, other rates arelinearly resampled.

Semantics

Volume curve: per-stream volume applies as (v/100)² at mix time —perceptually even slider response. Changes take effect within one mixerperiod (~12.5 ms).
Mute drains: a muted stream's buffered audio is discarded, sounmuting never plays stale audio.
Buffering: 1 s ring per stream; overflowing clients get their oldestaudio dropped (underrun_periods/buffered_ms visible in streams).
follows_default: streams opened without an explicit device followthe default output (and fire StreamChanged when it changes). ExplicitMoveStream/OPEN-device stops following.
Sinks: one output device is opened (ALSA plughw) per device thatstreams target; idle sinks close after 5 s. Moving a stream takes effectat the next period — game→headphones + music→speakers work simultaneously.
Latency: playback_latency_ms (default 50) per sink.
Metering: output levels are measured from the actual mixed signal —SubscribeLevels meters now reflect playback in both ALSA and demo mode.
Permissions

Same rules as the control plane: root, the daemon's uid (dev), and themitos-audio group (sudo groupadd -r mitos-audio; sudo usermod -aG mitos-audio <user>).

Rust (libmitos-audio)

let client = AudioClient::connect("/run/mitos/audio.sock").await?;let mut stream = client.open_playback("my-app").await?;   // 48k stereo s16loop {    let samples: Vec<i16> = produce_100ms_of_audio();    stream.write_i16(&samples).await?;}stream.close().await?;
Any language (raw socket, Python)

import socket, struct, json, maths = socket.socket(socket.AF_UNIX)s.connect("/run/mitos/audio-data.sock")def frame(t, payload=b""):    s.sendall(struct.pack("<IB", len(payload), t) + payload)frame(0x01, json.dumps({"application": "python-demo",     "sample_rate": 48000, "channels": 2, "format": "s16le"}).encode())print(s.recv(4096))  # OPEN_ACK {"stream_id": "s-1"}for _ in range(100):  # 10 s of 440 Hz    chunk = b""    for i in range(4800):                       # 100 ms        v = int(0.4 * 32767 * math.sin(2 * math.pi * 440 * i / 48000))        chunk += struct.pack("<hh", v, v)       # L, R    frame(0x02, chunk)frame(0x03)
Limits (v0.4) — on the roadmap

Capture (recording) streams are model-only — no data-plane capture yet.
Linear (not band-limited) resampler — slight aliasing at high frequencies.
Output path is stereo S16 via plughw; no drain/sync/cork commands yet.
Non-following streams whose target device vanishes stall (their ringdrops) until the device returns or they are moved.