//! libmitos-audio — typed async client for the mitos-audio daemon.
//!
//! ```no_run
//! use libmitos_audio::{AudioClient, ClientEvent};
//! # #[tokio::main]
//! # async fn main() -> Result<(), libmitos_audio::ClientError> {
//! let client = AudioClient::connect("/run/mitos/audio.sock").await?;
//! let state = client.get_state().await?;
//! println!("volume: {}%", state.master_volume);
//! # Ok(())
//! # }
//! ```
//!
//! Designed for GUI clients (mitos-gui, mitos-settings):
//! - `AudioClient` is `Send + Sync` — share it via `Arc`.
//! - Requests auto-reconnect (one transparent retry).
//! - `subscribe()` runs a monitor task that reconnects with backoff
//!   forever and emits `ClientEvent::Resync` after every (re)connect,
//!   telling you to refetch state.

mod client;
mod connection;

pub mod error;
pub mod events;
pub mod types;
mod levels;
mod playback;

pub use playback::PlaybackStream;
pub use levels::LevelStream;
pub use client::{AudioClient, ClientConfig};
pub use error::ClientError;
pub use events::{ClientEvent, EventStream};
pub use types::*;