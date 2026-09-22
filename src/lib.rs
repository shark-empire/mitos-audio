//! mitos-audio — MITOS system audio management service.
//!
//! Sits *above* ALSA (it is not a driver) and *below* user-facing clients
//! (mitos-gui, mitos-settings, applications), exposing a JSON-lines IPC API
//! over a Unix domain socket.

pub mod config;
pub mod daemon;
pub mod devices;
pub mod errors;
pub mod ipc;
pub mod manager;
pub mod streams;
pub mod engine;
pub mod volume;
pub mod backend;   // ← add this
pub mod monitoring;
pub mod persistence;
pub mod routing;