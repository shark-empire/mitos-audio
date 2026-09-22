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
    /// Reload routing rules from routing.toml
    ReloadRouting,
    /// Live level meters (dedicated levels subscription, ~10 Hz)
    Watch,

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
        }
        Cmd::MicGain { gain } => { let res = call(&sock, "SetMicrophoneGain", json!({ "gain": gain })).await?;
                                   println!("microphone gain set to {} dB", res["gain_db"]); }
        Cmd::MicMute   => { call(&sock, "MuteMicrophone", json!({ "mute": true })).await?;  println!("microphone muted"); }
        Cmd::MicUnmute => { call(&sock, "MuteMicrophone", json!({ "mute": false })).await?; println!("microphone unmuted"); }

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
        Cmd::ReloadRouting => {
            let res = call(&sock, "ReloadRouting", json!({})).await?;
            println!("routing reloaded: {} rule(s)", res["rules"]);
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
    println!("{:<14} {:<24} {:<12} {:<9} {:<12} {:>8}",
             "ID", "NAME", "KIND", "DIRECT", "STATE", "VOLUME");
    for d in &devices {
        println!("{:<14} {:<24} {:<12} {:<9} {:<12} {:>7}%",
            d.id, d.name,
            format!("{:?}", d.kind).to_lowercase(),
            format!("{:?}", d.direction).to_lowercase(),
            format!("{:?}", d.state).to_lowercase(),
            d.volume);
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