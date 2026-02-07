use std::collections::HashMap;
use std::io;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};

/// LSP write transport
pub struct LspWriter {
    stdin: ChildStdin,
}

impl LspWriter {
    pub fn new(stdin: ChildStdin) -> Self {
        Self { stdin }
    }

    pub async fn write_message(&mut self, msg: &serde_json::Value) -> io::Result<()> {
        let content = serde_json::to_string(msg)?;
        let content_bytes = content.as_bytes();
        let header = format!("Content-Length: {}\r\n\r\n", content_bytes.len());

        self.stdin.write_all(header.as_bytes()).await?;
        self.stdin.write_all(content_bytes).await?;
        self.stdin.flush().await?;

        Ok(())
    }
}

/// LSP read transport
pub struct LspReader {
    stdout: BufReader<ChildStdout>,
}

impl LspReader {
    pub fn new(stdout: ChildStdout) -> Self {
        Self {
            stdout: BufReader::new(stdout),
        }
    }

    pub async fn read_message(&mut self) -> io::Result<serde_json::Value> {
        let mut headers: HashMap<String, String> = HashMap::new();
        let mut line = String::new();

        loop {
            line.clear();
            self.stdout.read_line(&mut line).await?;

            if line == "\r\n" || line == "\n" {
                break;
            }

            if let Some((key, value)) = line.trim().split_once(':') {
                headers.insert(key.trim().to_lowercase(), value.trim().to_string());
            }
        }

        let content_length: usize = headers
            .get("content-length")
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Missing Content-Length"))?
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid Content-Length"))?;

        let mut buffer = vec![0u8; content_length];
        self.stdout.read_exact(&mut buffer).await?;

        serde_json::from_slice(&buffer)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}
