//! Play a 440 Hz sine through mitos-audio for five seconds.
//! MITOS_AUDIO_SOCK=/tmp/mitos-dev/audio.sock \
//!   cargo run -p libmitos-audio --example sine

use std::f64::consts::TAU;
use std::time::Duration;

use libmitos_audio::AudioClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket = std::env::var("MITOS_AUDIO_SOCK")
        .unwrap_or_else(|_| "/run/mitos/audio.sock".to_string());

    let client = AudioClient::connect(&socket).await?;
    let mut stream = client.open_playback("mitos-sine").await?;
    println!("stream {} — 440 Hz for 5 s (Ctrl+C to stop)", stream.stream_id());

    let chunk_frames = 4800; // 100 ms
    let mut frame_index: f64 = 0.0;
    for _ in 0..50 {
        let mut samples = Vec::with_capacity(chunk_frames * 2);
        for _ in 0..chunk_frames {
            let s = (0.4 * (TAU * 440.0 * frame_index / 48000.0).sin() * 32767.0) as i16;
            samples.push(s); // L
            samples.push(s); // R
            frame_index += 1.0;
        }
        stream.write_i16(&samples).await?;
        tokio::time::sleep(Duration::from_millis(95)).await;
    }
    stream.close().await?;
    println!("done");
    Ok(())
}