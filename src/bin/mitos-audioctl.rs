use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use mitos_audio::devices::device::{Device, Direction};
use mitos_audio::ipc::messages::Response;
use mitos_audio::streams::stream::AudioStream;

#[derive(Parser)]
#[command(name = "mitos-audioctl", version, about = "Control the mitos-audio service")]
struct Cli {
    #[arg(long, default_value = "/run/mitos/audio.sock")]
    socket: String,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List all audio devices
    Devices,
    /// List output devices
    Outputs,
    /// List input devices
    Inputs,
    /// Show or set the default output (`output headphones`)
    Output { id: Option<String> },
    /// Show or set the default input (`input usb-mic`)
    Input { id: Option<String> },
    /// Show or set the master volume (`volume 80`)
    Volume { value: Option<u32> },
    /// Mute the master output
    Mute,
    /// Unmute the master output
    Unmute,
    /// List active audio streams
    Streams,
    /// List available profiles
    Profiles,
    /// Activate a profile (`profile stereo`)
    Profile { name: String },
    /// Show microphone status
    Microphone,
    /// Set microphone gain in dB
    #[command(name = "mic-gain")]
    MicGain { gain: i32 },
    /// Mute the microphone
    #[command(name = "mic-mute")]
    MicMute,
    /// Unmute the microphone
    #[command(name = "mic-unmute")]
    MicUnmute,
    /// Toggle microphone DSP stages (any flag omitted is left unchanged)
    #[command(name = "mic-dsp")]
    MicDsp {
        #[arg(long)]
        noise_suppression: Option<bool>,
        #[arg(long)]
        echo_cancellation: Option<bool>,
        #[arg(long)]
        agc: Option<bool>,
    },
    /// Show output/input levels
    Levels,
    /// Subscribe to live events (Ctrl+C to stop)
    Monitor,
    /// Check the daemon is alive
    Ping,
    Rescan,
    NewStream {
        application: String,
        #[arg(long)]
        device: Option<String>,
        #[arg(long)]
        kind: Option<String>,   // playback | recording | capture | monitoring
    },
    /// Remove a stream by id
    RmStream { id: String },
    /// Move a stream to another device or speaker group
    #[command(name = "move-stream")]
    MoveStream { stream_id: String, target: String },
    /// List speaker groups (synchronized multi-speaker outputs)
    Groups,
    /// Create a group: group-create "Living Room" speakers hdmi0
    #[command(name = "group-create")]
    GroupCreate {
        name: String,
        #[arg(required = true)]
        members: Vec<String>,
    },
    /// Delete a speaker group
    #[command(name = "group-delete")]
    GroupDelete { id: String },
    /// Add a device to a group (--latency-ms: that device's output latency)
    #[command(name = "group-add")]
    GroupAdd {
        id: String,
        device: String,
        #[arg(long)]
        latency_ms: Option<u32>,
    },
    /// Remove a device from a group
    #[command(name = "group-remove")]
    GroupRemove { id: String, device: String },
    /// Set a member's output latency in ms; faster members are delayed to match
    #[command(name = "group-latency")]
    GroupLatency { id: String, device: String, latency_ms: u32 },
    /// Reload routing rules from routing.toml
    ReloadRouting,
    /// Reload application policy from policy.toml
    ReloadPolicy,
    /// Show effects (EQ/compressor/limiter) status for a device
    Effects {
        #[arg(long)]
        device: Option<String>,
    },
    /// Turn effects on for a device
    #[command(name = "effects-on")]
    EffectsOn {
        #[arg(long)]
        device: Option<String>,
    },
    /// Turn effects off for a device
    #[command(name = "effects-off")]
    EffectsOff {
        #[arg(long)]
        device: Option<String>,
    },
    /// Apply a named preset: flat, music, movie, game, voice, podcast
    #[command(name = "effects-preset")]
    EffectsPreset {
        preset: String,
        #[arg(long)]
        device: Option<String>,
    },
    /// Set all 10 EQ band gains in dB (31 62 125 250 500 1k 2k 4k 8k 16k Hz)
    Eq {
        #[arg(num_args = 10)]
        bands: Vec<f32>,
        #[arg(long)]
        device: Option<String>,
    },
    /// Live level meters (dedicated levels subscription, ~10 Hz)
    Watch,
    
    Tone {
        #[arg(default_value_t = 440.0)]
        freq: f64,
        #[arg(default_value_t = 3.0)]
        secs: f64,
        /// Target device id (default: follow the default output)
        #[arg(long)]
        device: Option<String>,
    },

}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let sock = cli.socket.clone();

    match cli.command {
        
        Cmd::Devices => { let res = call(&sock, "ListDevices", json!({})).await?; print_devices(&res, None)?; }
        Cmd::Outputs => { let res = call(&sock, "ListDevices", json!({})).await?; print_devices(&res, Some(Direction::Output))?; }
        Cmd::Inputs  => { let res = call(&sock, "ListDevices", json!({})).await?; print_devices(&res, Some(Direction::Input))?; }

        Cmd::Rescan => {
        let res = call(&sock, "Rescan", json!({})).await?;
        print_devices(&res, None)?;
          }
        Cmd::Output { id } => match id {
            None => { let res = call(&sock, "GetDefaults", json!({})).await?;
                      println!("default output: {}", res["default_output"].as_str().unwrap_or("(none)")); }
            Some(id) => { let res = call(&sock, "SetDefaultOutput", json!({ "id": id })).await?;
                          println!("default output set to {}", res["default_output"].as_str().unwrap_or("?")); }
        },
        Cmd::Input { id } => match id {
            None => { let res = call(&sock, "GetDefaults", json!({})).await?;
                      println!("default input: {}", res["default_input"].as_str().unwrap_or("(none)")); }
            Some(id) => { let res = call(&sock, "SetDefaultInput", json!({ "id": id })).await?;
                          println!("default input set to {}", res["default_input"].as_str().unwrap_or("?")); }
        },

        Cmd::Volume { value } => match value {
            None => { let res = call(&sock, "GetVolume", json!({})).await?;
                      let muted = if res["muted"].as_bool().unwrap_or(false) { " (muted)" } else { "" };
                      println!("volume: {}%{}", res["volume"], muted); }
            Some(v) => { let res = call(&sock, "SetVolume", json!({ "volume": v })).await?;
                         println!("volume set to {}%", res["volume"]); }
        },
        Cmd::Mute   => { call(&sock, "Mute", json!({})).await?; println!("muted"); }
        Cmd::Unmute => { call(&sock, "Unmute", json!({})).await?; println!("unmuted"); }

        Cmd::Streams => {
            let res = call(&sock, "ListStreams", json!({})).await?;
            let streams: Vec<AudioStream> = serde_json::from_value(res["streams"].clone())?;
            println!("{:<10} {:<18} {:<11} {:<14} {:>8} {:>7}",
                     "ID", "APPLICATION", "KIND", "DEVICE", "VOLUME", "MUTED");
            for s in &streams {
                println!("{:<10} {:<18} {:<11} {:<14} {:>7}% {:>7}",
                    s.id, s.application,
                    format!("{:?}", s.kind).to_lowercase(),
                    s.device, s.volume, s.muted);
            }
        }

        Cmd::Profiles => {
            let res = call(&sock, "ListProfiles", json!({})).await?;
            let active = res["active"].as_str().unwrap_or("");
            if let Some(list) = res["profiles"].as_array() {
                for p in list.iter().filter_map(|p| p.as_str()) {
                    let marker = if p == active { "  <- active" } else { "" };
                    println!(" {p}{marker}");
                }
            }
        }
        Cmd::Profile { name } => { call(&sock, "SetProfile", json!({ "profile": name })).await?; println!("profile set to {name}"); }

        Cmd::Microphone => {
            let res = call(&sock, "GetMicrophone", json!({})).await?;
            println!("microphone : {}", res["device"]["name"].as_str().unwrap_or("(none)"));
            println!("muted      : {}", res["muted"]);
            println!("gain       : {} dB", res["gain_db"]);
            println!("noise sup. : {}", res["noise_suppression"]);
            println!("echo canc. : {}", res["echo_cancellation"]);
            println!("agc        : {}", res["agc"]);
        }
        Cmd::MicGain { gain } => { let res = call(&sock, "SetMicrophoneGain", json!({ "gain": gain })).await?;
                                   println!("microphone gain set to {} dB", res["gain_db"]); }
        Cmd::MicMute   => { call(&sock, "MuteMicrophone", json!({ "mute": true })).await?;  println!("microphone muted"); }
        Cmd::MicUnmute => { call(&sock, "MuteMicrophone", json!({ "mute": false })).await?; println!("microphone unmuted"); }
        Cmd::MicDsp { noise_suppression, echo_cancellation, agc } => {
            let res = call(&sock, "SetMicrophoneProcessing", json!({
                "noise_suppression": noise_suppression,
                "echo_cancellation": echo_cancellation,
                "agc": agc,
            })).await?;
            println!("noise suppression : {}", res["noise_suppression"]);
            println!("echo cancellation : {}", res["echo_cancellation"]);
            println!("agc               : {}", res["agc"]);
        }

        Cmd::Levels => { let res = call(&sock, "GetLevels", json!({})).await?;
                         println!("{}", serde_json::to_string_pretty(&res)?); }

        Cmd::Monitor => {
            let stream = UnixStream::connect(&sock).await?;
            let (mut read_half, mut write_half) = stream.into_split();
            let request = json!({ "id": 1, "command": "SubscribeEvents", "params": {} });
            write_half.write_all(request.to_string().as_bytes()).await?;
            write_half.write_all(b"\n").await?;
            let mut reader = BufReader::new(read_half);
            let mut line = String::new();
            println!("monitoring mitos-audio events (Ctrl+C to stop)");
            loop {
                line.clear();
                let n = reader.read_line(&mut line).await?;
                if n == 0 { break; }
                print!("{}", line);
            }
        }
        
        Cmd::NewStream { application, device, kind } => {
            let mut params = json!({ "application": application });
            if let Some(d) = device { params["device"] = json!(d); }
            if let Some(k) = kind { params["kind"] = json!(k); }
            let res = call(&sock, "CreateStream", params).await?;
            println!(
                "stream {} created for '{}' on {} (volume {}%)",
                res["stream"]["id"], res["stream"]["application"],
                res["stream"]["device"], res["stream"]["volume"]
            );
        }
        Cmd::RmStream { id } => {
            call(&sock, "DestroyStream", json!({ "stream_id": id })).await?;
            println!("stream {id} removed");
        }
        Cmd::MoveStream { stream_id, target } => {
            call(&sock, "MoveStream", json!({ "stream_id": stream_id, "device_id": target })).await?;
            println!("stream {stream_id} moved to {target}");
        }
        Cmd::Groups => {
            let res = call(&sock, "ListGroups", json!({})).await?;
            match res["groups"].as_array() {
                Some(groups) if !groups.is_empty() => {
                    for g in groups {
                        println!("{}  \"{}\"", g["id"].as_str().unwrap_or("-"), g["name"].as_str().unwrap_or("-"));
                        if let Some(members) = g["members"].as_array() {
                            for m in members {
                                println!(
                                    "    {:<14} latency {} ms",
                                    m["device_id"].as_str().unwrap_or("-"),
                                    m["latency_ms"]
                                );
                            }
                        }
                    }
                }
                _ => println!("no speaker groups"),
            }
        }
        Cmd::GroupCreate { name, members } => {
            let res = call(&sock, "CreateGroup", json!({ "name": name, "members": members })).await?;
            let id = res["id"].as_str().unwrap_or("");
            println!("group {id} created — target it with: mitos-audioctl tone --device {id}");
        }
        Cmd::GroupDelete { id } => {
            let res = call(&sock, "DeleteGroup", json!({ "id": id })).await?;
            if res["removed"].as_bool().unwrap_or(false) {
                println!("group {id} deleted");
            } else {
                println!("no such group: {id}");
            }
        }
        Cmd::GroupAdd { id, device, latency_ms } => {
            let mut params = json!({ "id": id, "device_id": device });
            if let Some(l) = latency_ms {
                params["latency_ms"] = json!(l);
            }
            let res = call(&sock, "AddGroupMember", params).await?;
            println!("group {id} now has {} member(s)", res["members"]);
        }
        Cmd::GroupRemove { id, device } => {
            let res = call(&sock, "RemoveGroupMember", json!({ "id": id, "device_id": device })).await?;
            println!("group {id} now has {} member(s)", res["members"]);
        }
        Cmd::GroupLatency { id, device, latency_ms } => {
            call(
                &sock,
                "SetGroupMemberLatency",
                json!({ "id": id, "device_id": device, "latency_ms": latency_ms }),
            )
            .await?;
            println!("{device} in group {id}: latency set to {latency_ms} ms");
        }
        
        Cmd::Tone { freq, secs, device } => {
            let data_socket = mitos_audio::engine::protocol::derive_data_socket(&sock);
            let mut stream = UnixStream::connect(&data_socket).await?;

            let mut open = json!({
                "application": "mitos-tone",
                "sample_rate": 48000,
                "channels": 2,
                "format": "s16le",
            });
            if let Some(d) = device {
                open["device"] = json!(d);
            }
            df_write(&mut stream, 0x01, open.to_string().as_bytes()).await?;
            let (kind, payload) = df_read(&mut stream).await?;
            if kind == 0x82 {
                let e: Value = serde_json::from_slice(&payload)?;
                return Err(format!("[{}] {}", e["code"], e["message"]).into());
            }
            let ack: Value = serde_json::from_slice(&payload)?;
            println!(
                "playing {freq:.0} Hz for {secs:.0} s (stream {}) — Ctrl+C to stop",
                ack["stream_id"].as_str().unwrap_or("?")
            );

            let chunk_frames = 4800usize; // 100 ms
            let chunks = (secs * 10.0).ceil() as usize;
            let mut phase: f64 = 0.0;
            for _ in 0..chunks {
                let mut buf = Vec::with_capacity(chunk_frames * 4);
                for _ in 0..chunk_frames {
                    let s = (0.5
                        * (std::f64::consts::TAU * freq * phase / 48000.0).sin()
                        * 32767.0) as i16;
                    buf.extend_from_slice(&s.to_le_bytes()); // L
                    buf.extend_from_slice(&s.to_le_bytes()); // R
                    phase += 1.0;
                }
                df_write(&mut stream, 0x02, &buf).await?;
                // Slightly ahead of realtime — the daemon's ring absorbs jitter.
                tokio::time::sleep(std::time::Duration::from_millis(95)).await;
            }
            df_write(&mut stream, 0x03, &[]).await?;
            println!("done");
        }
        Cmd::ReloadRouting => {
            let res = call(&sock, "ReloadRouting", json!({})).await?;
            println!("routing reloaded: {} rule(s)", res["rules"]);
        }
        Cmd::ReloadPolicy => {
            let res = call(&sock, "ReloadPolicy", json!({})).await?;
            println!("policy reloaded: {} rule(s)", res["rules"]);
        }
        Cmd::Effects { device } => {
            let res = call(&sock, "GetEffects", json!({ "device": device })).await?;
            println!("device  : {}", res["device"].as_str().unwrap_or("-"));
            println!("enabled : {}", res["enabled"]);
            println!("preset  : {}", res["preset"].as_str().unwrap_or("-"));
            if let Some(bands) = res["bands"].as_array() {
                let freqs = ["31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];
                for (f, b) in freqs.iter().zip(bands.iter()) {
                    println!("  {:>4}Hz  {:+.1} dB", f, b.as_f64().unwrap_or(0.0));
                }
            }
        }
        Cmd::EffectsOn { device } => {
            call(&sock, "SetEffectsEnabled", json!({ "device": device, "enabled": true })).await?;
            println!("effects on");
        }
        Cmd::EffectsOff { device } => {
            call(&sock, "SetEffectsEnabled", json!({ "device": device, "enabled": false })).await?;
            println!("effects off");
        }
        Cmd::EffectsPreset { preset, device } => {
            let res = call(&sock, "SetEffectsPreset", json!({ "device": device, "preset": preset })).await?;
            println!("preset set to {}", res["preset"]);
        }
        Cmd::Eq { bands, device } => {
            if bands.len() != 10 {
                return Err("expected exactly 10 band values".into());
            }
            call(&sock, "SetEqualizerBands", json!({ "device": device, "bands": bands })).await?;
            println!("equalizer updated (preset: custom)");
        }
        Cmd::Watch => {
            let stream = UnixStream::connect(&sock).await?;
            let (mut read_half, mut write_half) = stream.into_split();
            let request = json!({ "id": 1, "command": "SubscribeLevels", "params": {} });
            write_half.write_all(request.to_string().as_bytes()).await?;
            write_half.write_all(b"\n").await?;
            let mut reader = BufReader::new(read_half);
            let mut line = String::new();
            println!("mitos-audio levels (Ctrl+C to stop)");
            loop {
                line.clear();
                if reader.read_line(&mut line).await? == 0 { break; }
                let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
                if v.get("event").and_then(Value::as_str) != Some("LevelChanged") { continue; }
                let d = &v["data"];
                let out = d["output_level"].as_f64().unwrap_or(0.0);
                let inp = d["input_level"].as_f64().unwrap_or(0.0);
                let peak = d["peak"].as_f64().unwrap_or(0.0);
                let clip = d["clipping"].as_bool().unwrap_or(false);
                print!(
                    "\r\x1b[2K OUT {} {:>3.0}%   IN {} {:>3.0}%   peak {:>3.0}%{}",
                    bar(out), out * 100.0,
                    bar(inp), inp * 100.0,
                    peak * 100.0,
                    if clip { "   ⚠ CLIPPING" } else { "" }
                );
                use std::io::Write;
                std::io::stdout().flush().ok();
            }
            println!();
        }

        Cmd::Ping => { call(&sock, "Ping", json!({})).await?; println!("mitos-audio is alive"); }
    }
    Ok(())
}

async fn call(socket: &str, command: &str, params: Value)
    -> Result<Value, Box<dyn std::error::Error>>
{
    let mut stream = UnixStream::connect(socket).await?;
    let request = json!({ "id": 1, "command": command, "params": params });
    stream.write_all(request.to_string().as_bytes()).await?;
    stream.write_all(b"\n").await?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let response: Response = serde_json::from_str(line.trim())?;
    if response.ok {
        Ok(response.result.unwrap_or(Value::Null))
    } else {
        let err = response.error.unwrap();
        Err(format!("[{}] {}", err.code, err.message).into())
    }
}

fn print_devices(res: &Value, filter: Option<Direction>) -> Result<(), Box<dyn std::error::Error>> {
    let devices: Vec<Device> = serde_json::from_value(res["devices"].clone())?;
    let devices: Vec<Device> = devices.into_iter()
        .filter(|d| match filter {
            None => true,
            Some(Direction::Input) => d.direction != Direction::Output,
            Some(_) => d.direction != Direction::Input,
        })
        .collect();
    println!("{:<14} {:<24} {:<12} {:<9} {:<12} {:>8}  {}",
             "ID", "NAME", "KIND", "DIRECT", "STATE", "VOLUME", "CODEC");
    for d in &devices {
        println!("{:<14} {:<24} {:<12} {:<9} {:<12} {:>7}%  {}",
            d.id, d.name,
            format!("{:?}", d.kind).to_lowercase(),
            format!("{:?}", d.direction).to_lowercase(),
            format!("{:?}", d.state).to_lowercase(),
            d.volume,
            d.codec.as_deref().unwrap_or("-"));
    }
    println!("\ndefault output: {}    default input: {}",
        res["default_output"].as_str().unwrap_or("-"),
        res["default_input"].as_str().unwrap_or("-"));
    Ok(())
}

fn bar(v: f64) -> String {
    const WIDTH: usize = 24;
    let filled = (v.clamp(0.0, 1.0) * WIDTH as f64).round() as usize;
    format!("{}{}", "█".repeat(filled), "░".repeat(WIDTH - filled))
}

async fn df_write(stream: &mut UnixStream, kind: u8, payload: &[u8])
    -> Result<(), Box<dyn std::error::Error>>
{
    use tokio::io::AsyncWriteExt;
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.push(kind);
    frame.extend_from_slice(payload);
    stream.write_all(&frame).await?;
    Ok(())
}

async fn df_read(stream: &mut UnixStream)
    -> Result<(u8, Vec<u8>), Box<dyn std::error::Error>>
{
    use tokio::io::AsyncReadExt;
    let mut header = [0u8; 5];
    stream.read_exact(&mut header).await?;
    let len = u32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize;
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await?;
    Ok((header[4], payload))
}