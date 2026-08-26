//! Local Windows control plane for commands that belong to the already-running daemon.
//!
//! A second `conduit-daemon.exe send <path>` process must not open another peer listener or
//! another relay park. It sends one bounded request over a local named pipe and exits; the resident
//! daemon keeps ownership of the one Noise session. This is also the IPC seam a future Explorer or
//! Fluent UI can reuse without becoming a transport owner.

use anyhow::{bail, Context, Result};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::file;

const PIPE: &str = r"\\.\pipe\Conduit.Send.v1";
const MAX_REQUEST: usize = 32 * 1024;
const CLIENT_RETRIES: usize = 20;
const CLIENT_RETRY: Duration = Duration::from_millis(50);

/// Serves local send requests forever. Each connected client is tiny and independent; file bytes
/// never enter this pipe, only the canonical path that the resident daemon will open itself.
pub async fn serve(tx: mpsc::Sender<PathBuf>) -> Result<()> {
    loop {
        let pipe = ServerOptions::new()
            .create(PIPE)
            .context("creating the Conduit local control pipe")?;
        pipe.connect().await.context("accepting a Conduit control client")?;
        let tx = tx.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(pipe, tx).await {
                warn!(error = %e, "local send request failed");
            }
        });
    }
}

async fn handle(mut pipe: NamedPipeServer, tx: mpsc::Sender<PathBuf>) -> Result<()> {
    let outcome = read_path(&mut pipe).await.and_then(|path| {
        let path = file::validate_outbound(&path)?;
        tx.try_send(path.clone())
            .map_err(|e| anyhow::anyhow!("outbound file queue is unavailable: {e}"))?;
        Ok(path)
    });

    match outcome {
        Ok(path) => {
            pipe.write_all(b"OK\n").await?;
            info!(path = %path.display(), "file queued for phone");
        }
        Err(e) => {
            let reply = format!("ERR {e:#}\n");
            // The request is already refused; failure to explain it to a client is not a reason
            // to keep the pipe instance around.
            let _ = pipe.write_all(reply.as_bytes()).await;
        }
    }
    let _ = pipe.shutdown().await;
    Ok(())
}

async fn read_path(pipe: &mut NamedPipeServer) -> Result<PathBuf> {
    let len = pipe.read_u32().await? as usize;
    if len == 0 || len > MAX_REQUEST {
        bail!("invalid local send request length {len}");
    }
    let mut bytes = vec![0u8; len];
    pipe.read_exact(&mut bytes).await?;
    let path = String::from_utf8(bytes).context("send path is not UTF-8")?;
    Ok(PathBuf::from(path))
}

/// CLI side of `conduit-daemon.exe send <path>`.
pub async fn queue(path: &Path) -> Result<PathBuf> {
    let path = file::validate_outbound(path)?;
    let text = path.to_str().context("the path cannot be represented as UTF-8")?;
    let bytes = text.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_REQUEST {
        bail!("path is too long for the local send request");
    }

    let mut client = None;
    let mut last = None;
    for _ in 0..CLIENT_RETRIES {
        match ClientOptions::new().open(PIPE) {
            Ok(pipe) => {
                client = Some(pipe);
                break;
            }
            Err(e) if pipe_busy(&e) => {
                last = Some(e);
                tokio::time::sleep(CLIENT_RETRY).await;
            }
            Err(e) => return Err(e).context("opening the Conduit local control pipe"),
        }
    }
    let mut client = client.ok_or_else(|| {
        anyhow::anyhow!(
            "Conduit daemon is not accepting local requests{}",
            last.map(|e| format!(": {e}")).unwrap_or_default()
        )
    })?;

    client.write_u32(bytes.len() as u32).await?;
    client.write_all(bytes).await?;
    let mut reply = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut reply))
        .await
        .context("timed out waiting for the resident Conduit daemon")??;
    let reply = String::from_utf8(reply).context("control response is not UTF-8")?;
    if reply == "OK\n" {
        Ok(path)
    } else if let Some(error) = reply.strip_prefix("ERR ") {
        bail!("{}", error.trim_end());
    } else {
        bail!("unexpected local control response {reply:?}");
    }
}

fn pipe_busy(error: &io::Error) -> bool {
    // ERROR_PIPE_BUSY. Windows maps it inconsistently across Rust versions, so preserve the raw
    // value rather than relying only on ErrorKind::WouldBlock.
    error.kind() == io::ErrorKind::WouldBlock || error.raw_os_error() == Some(231)
}
