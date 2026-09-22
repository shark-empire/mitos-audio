mitos-audio — Integration Guide

How external systems (mitos-gui, mitos-settings, applications, scripts)connect to mitos-audio.

1. The integration contract

All clients talk to mitos-audio over a single Unix domain socket:

Item	Value
Transport	Unix domain socket (SOCK_STREAM)
Path	/run/mitos/audio.sock
Encoding	UTF-8 JSON
Framing	one JSON object per line (JSONL)
Model	request/response plus pub/sub events
Dependencies	none — no D-Bus, no HTTP, no shared libraries
Any language that can open a Unix socket can integrate.

2. Where mitos-audio sits

Linux kernel → ALSA                 │            mitos-audio            ← this service                 │       IPC API (/run/mitos/audio.sock)   ┌──────────┬──────────────┬───────────────┬────────────────┐mitos-gui  mitos-settings  applications  mitos-audioctl
3. Quick test (no code required)

sudo systemctl start mitos-audiosocat - UNIX-CONNECT:/run/mitos/audio.sock{"id":1,"command":"GetState","params":{}}{"id":2,"command":"SetVolume","params":{"volume":60}}
Or use the CLI: mitos-audioctl devices, mitos-audioctl monitor.

4. Minimal client (Rust)

use serde_json::{json, Value};use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};use tokio::net::UnixStream;pub struct MitosAudio {    stream: UnixStream,}impl MitosAudio {    pub async fn connect() -> std::io::Result<Self> {        Ok(Self { stream: UnixStream::connect("/run/mitos/audio.sock").await? })    }    pub async fn call(&mut self, id: u64, command: &str, params: Value)        -> Result<Value, String>    {        let request = json!({ "id": id, "command": command, "params": params });        self.stream.write_all(request.to_string().as_bytes()).await.map_err(|e| e.to_string())?;        self.stream.write_all(b"\n").await.map_err(|e| e.to_string())?;        let mut reader = BufReader::new(&mut self.stream);        let mut line = String::new();        reader.read_line(&mut line).await.map_err(|e| e.to_string())?;        let response: Value = serde_json::from_str(line.trim()).map_err(|e| e.to_string())?;        if response["ok"].as_bool().unwrap_or(false) {            Ok(response["result"].cloned().unwrap_or(Value::Null))        } else {            Err(response["error"]["message"].as_str().unwrap_or("unknown error").to_string())        }    }}// Usage:// let mut audio = MitosAudio::connect().await?;// let state = audio.call(1, "GetState", json!({})).await?;// let volume = state["master_volume"].as_u64().unwrap_or(0);
5. Subscribing to events (Rust)

use serde_json::json;use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};use tokio::net::UnixStream;pub async fn monitor() -> std::io::Result<()> {    let stream = UnixStream::connect("/run/mitos/audio.sock").await?;    let (mut read_half, mut write_half) = stream.into_split();    let request = json!({ "id": 1, "command": "SubscribeEvents", "params": {} });    write_half.write_all(request.to_string().as_bytes()).await?;    write_half.write_all(b"\n").await?;    let mut reader = BufReader::new(read_half);    let mut line = String::new();    loop {        line.clear();        if reader.read_line(&mut line).await? == 0 { break; }        println!("{}", line.trim());   // {"event":"VolumeChanged","data":{"volume":60}}    }    Ok(())}
Important: on a subscribed connection, events and responses share thesame line stream. Distinguish them by key: responses carry "id", eventscarry "event". For simplicity, clients may use one connection forcontrol and one for events.

6. mitos-gui integration

Recommended pattern for GUI panels (mixer, output switcher, volume OSD):

On startup → GetState (one roundtrip) → build the UI model.
Subscribe → SubscribeEvents on a second connection → apply deltas.
Meters → poll GetLevels every 50–100 ms (until streaming meteringevents land; see roadmap).
Reconnect with backoff; on reconnect, re-issue GetState and rebuild.
Example: volume OSD reacting to hardware/media-key changes:

// pseudo-code inside the GUI event loopmatch event {    Event::VolumeChanged { device: None, volume } => osd.show_volume(volume),    Event::MuteChanged { muted: true }            => osd.show_mute_icon(),    Event::DefaultOutputChanged { id }            => notify(format!("Output → {id}")),    _ => {}}
Per-application mixer: render one row per entry of streams fromGetState; update rows on StreamAdded / StreamRemoved /StreamChanged; apply changes with SetStreamVolume, SetStreamMute,MoveStream.

7. mitos-settings integration

Same protocol. Differences from the GUI:

Long-lived connection, reconnect with exponential backoff.
Persist user preferences locally; mitos-audio owns runtime state only(until the persistence layer lands — see roadmap).
The settings page is a natural place to expose ListProfiles /SetProfile and default-device pickers (SetDefaultOutput /SetDefaultInput).
8. Application integration

Applications appear as streams (application, per-app volume, mute,device) — this is what powers per-app mixing and routing.
Control plane: this socket API (change your own stream's volume etc.).
Audio plane (actual samples): applications currently open audio throughthe lower stack; a native libmitos-audio playback API is on the roadmapand will route samples through mitos-audio directly.
Interim option for wide app compatibility: a PulseAudio/PIPEWIREcompatibility shim translating app streams into mitos-audio streams.
9. Script & shell integration

mitos-audioctl volume 80mitos-audioctl output headphonesmitos-audioctl mic-mutemitos-audioctl monitor | jq 'select(.event == "DeviceAdded")'
10. systemd

Install services/mitos-audio.service. The unit creates /run/mitos(RuntimeDirectory=mitos), so no manual directory setup is needed.

11. Permissions & security

Socket is created 0660; only permitted peers may connect.
v0.1: SO_PEERCRED check — root and the daemon's own uid.
Next: membership in a mitos-audio group + per-application permissiongrants from policy.toml (microphone access in particular).
12. Compatibility rules

Protocol changes are additive only within a minor series.
Clients must ignore unknown fields and unknown events.
GetState returns "version" — check it, don't parse it strictly.


4. Rust clients — use libmitos-audio (recommended)

The repository ships a typed client crate (client/, package namelibmitos-audio). It implements the entire IPC surface with reconnecthandling baked in — this is what mitos-gui and mitos-settings should useinstead of hand-rolled sockets:

AudioClient::connect() — one shared Arc<AudioClient> per process
get_state(), list_devices(), set_volume(), set_default_output(), …— every command, fully typed
subscribe() — background event monitor with exponential-backoffreconnect and a Resync event telling you when to refetch state
See client/README.md for the full API tour and the recommended GUI loop(get_state → subscribe → react to Resync by refetching).

For other languages, the raw-socket examples in the next section show thesame wire protocol from scratch.

Also append to §6 (mitos-gui integration):

In Rust, the recommended loop is one line shorter with the client crate:AudioClient::subscribe() delivers ClientEvent::Resync on every(re)connect — treat it as the signal to re-issue get_state(). Metersstill poll GetLevels every 50–100 ms until streaming metering lands.