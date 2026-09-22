//! One JSON object per line, over a Unix domain socket.

use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;

use crate::error::ClientError;

#[derive(Debug, Deserialize)]
pub(crate) struct WireResponse {
    #[allow(dead_code)]
    pub id: u64,
    pub ok: bool,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<WireError>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WireError {
    pub code: String,
    pub message: String,
}

pub(crate) struct Connection {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl Connection {
    pub async fn open(path: &Path) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(path).await?;
        let (read, write) = stream.into_split();
        Ok(Self { reader: BufReader::new(read), writer: write })
    }

    /// Fire a request and await its response.
    pub async fn request(
        &mut self,
        id: u64,
        command: &str,
        params: Value,
    ) -> Result<Value, ClientError> {
        self.send(id, command, params).await?;
        self.receive().await
    }

    /// Write a request line (used by the event monitor).
    pub async fn send(
        &mut self,
        id: u64,
        command: &str,
        params: Value,
    ) -> Result<(), ClientError> {
        let line = json!({ "id": id, "command": command, "params": params }).to_string();
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// Read and decode one response line.
    pub async fn receive(&mut self) -> Result<Value, ClientError> {
        let line = self.read_line().await?;
        let resp: WireResponse = serde_json::from_str(line.trim())?;
        match (resp.ok, resp.result, resp.error) {
            (true, Some(v), _) => Ok(v),
            (true, None, _) => Ok(Value::Null),
            (_, _, Some(e)) => Err(ClientError::Daemon { code: e.code, message: e.message }),
            (_, _, None) => Err(ClientError::Protocol("error response without body".into())),
        }
    }

    /// Read one raw line (event or response).
    pub async fn read_line(&mut self) -> Result<String, ClientError> {
        let mut buf = String::new();
        let n = self.reader.read_line(&mut buf).await?;
        if n == 0 {
            Err(ClientError::Disconnected)
        } else {
            Ok(buf)
        }
    }
}