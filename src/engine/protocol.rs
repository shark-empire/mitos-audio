//! Data-plane wire protocol: `[u32 payload_len][u8 type][payload]`,
//! little-endian, max payload 1 MiB. Full reference: docs/audio-plane.md.

use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_PAYLOAD: usize = 1 << 20;

// client → daemon
pub const MSG_OPEN: u8 = 0x01;
pub const MSG_DATA: u8 = 0x02;
pub const MSG_CLOSE: u8 = 0x03;
// daemon → client
pub const MSG_OPEN_ACK: u8 = 0x81;
pub const MSG_ERROR: u8 = 0x82;
pub const MSG_CLOSED: u8 = 0x83;

/// Derive the data-plane socket from the control socket
/// (`…/audio.sock` → `…/audio-data.sock`).
pub fn derive_data_socket(control: &str) -> String {
    match control.strip_suffix(".sock") {
        Some(stem) => format!("{stem}-data.sock"),
        None => format!("{control}-data"),
    }
}

pub struct Frame {
    pub kind: u8,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn encode(kind: u8, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(5 + payload.len());
        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buf.push(kind);
        buf.extend_from_slice(payload);
        buf
    }
}

pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Frame> {
    let mut header = [0u8; 5];
    reader.read_exact(&mut header).await?;
    let len = u32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize;
    let kind = header[4];
    if len > MAX_PAYLOAD {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "data-plane frame too large"));
    }
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    Ok(Frame { kind, payload })
}

pub async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    kind: u8,
    payload: &[u8],
) -> io::Result<()> {
    writer.write_all(&Frame::encode(kind, payload)).await
}

#[derive(Debug, serde::Deserialize)]
pub struct OpenRequest {
    pub application: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub format: String,
    #[serde(default)]
    pub device: Option<String>,
}