use mitos_audio::{config::AudioConfig, daemon};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config_path = std::env::var("MITOS_AUDIO_CONFIG")
        .unwrap_or_else(|_| "/etc/mitos/audio.toml".to_string());
    let config = AudioConfig::load_or_default(&config_path)?;

    tracing::info!(socket = %config.socket_path, "starting mitos-audio");
    daemon::run(config).await?;
    Ok(())
}