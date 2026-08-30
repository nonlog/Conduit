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
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use crate::file;

const PIPE: &str = r"\\.\pipe\Conduit.Send.v1";
const MAX_REQUEST: usize = 32 * 1024;
const CLIENT_RETRIES: usize = 20;
const CLIENT_RETRY: Duration = Duration::from_millis(50);
const REMOTE_RESULT_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const LOCAL_RESULT_TIMEOUT: Duration = Duration::from_secs(5);
/// Begins with NUL, which a valid Win32 path cannot contain, so old path-only clients remain valid.
const RELOAD_COMMAND: &str = "\0reload\0";

pub struct SendRequest {
    pub path: PathBuf,
    pub completion: oneshot::Sender<std::result::Result<(), String>>,
}

pub struct ReloadRequest {
    pub completion: oneshot::Sender<std::result::Result<(), String>>,
}

/// Serves local requests forever. The pipe remains backward compatible with the original
/// path-only sender: only a NUL-prefixed payload is interpreted as a daemon command.
pub async fn serve(
    send_tx: mpsc::Sender<SendRequest>,
    reload_tx: mpsc::Sender<ReloadRequest>,
) -> Result<()> {
    loop {
        let pipe = ServerOptions::new()
            .create(PIPE)
            .context("creating the Conduit local control pipe")?;
        pipe.connect()
            .await
            .context("accepting a Conduit control client")?;
        let send_tx = send_tx.clone();
        let reload_tx = reload_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(pipe, send_tx, reload_tx).await {
                warn!(error = %e, "local control request failed");
            }
        });
    }
}

async fn handle(
    mut pipe: NamedPipeServer,
    send_tx: mpsc::Sender<SendRequest>,
    reload_tx: mpsc::Sender<ReloadRequest>,
) -> Result<()> {
    let request = read_request(&mut pipe).await?;
    if request == RELOAD_COMMAND {
        let outcome: Result<()> = async {
            let (completion, done) = oneshot::channel();
            reload_tx
                .try_send(ReloadRequest { completion })
                .map_err(|e| anyhow::anyhow!("settings reload queue is unavailable: {e}"))?;
            match tokio::time::timeout(LOCAL_RESULT_TIMEOUT, done).await {
                Ok(Ok(Ok(()))) => Ok(()),
                Ok(Ok(Err(message))) => bail!("could not apply settings: {message}"),
                Ok(Err(_)) => bail!("daemon dropped the settings reload request"),
                Err(_) => bail!("timed out applying settings"),
            }
        }
        .await;
        match outcome {
            Ok(()) => pipe.write_all(b"OK\n").await?,
            Err(e) => {
                let reply = format!("ERR {e:#}\n");
                let _ = pipe.write_all(reply.as_bytes()).await;
            }
        }
        let _ = pipe.shutdown().await;
        return Ok(());
    }

    let outcome: Result<PathBuf> = async {
        let path = file::validate_outbound(Path::new(&request))?;
        let (completion, done) = oneshot::channel();
        send_tx
            .try_send(SendRequest {
                path: path.clone(),
                completion,
            })
            .map_err(|e| anyhow::anyhow!("outbound file queue is unavailable: {e}"))?;

        match tokio::time::timeout(REMOTE_RESULT_TIMEOUT, done).await {
            Ok(Ok(Ok(()))) => Ok(path),
            Ok(Ok(Err(message))) => bail!("phone did not publish the file: {message}"),
            Ok(Err(_)) => bail!("the Conduit session ended before the phone confirmed the file"),
            Err(_) => bail!("timed out waiting for the phone to publish the file"),
        }
    }
    .await;

    match outcome {
        Ok(path) => {
            pipe.write_all(b"OK\n").await?;
            info!(path = %path.display(), "phone confirmed file publication");
        }
        Err(e) => {
            let reply = format!("ERR {e:#}\n");
            let _ = pipe.write_all(reply.as_bytes()).await;
        }
    }
    let _ = pipe.shutdown().await;
    Ok(())
}

async fn read_request(pipe: &mut NamedPipeServer) -> Result<String> {
    let len = pipe.read_u32().await? as usize;
    if len == 0 || len > MAX_REQUEST {
        bail!("invalid local control request length {len}");
    }
    let mut bytes = vec![0u8; len];
    pipe.read_exact(&mut bytes).await?;
    String::from_utf8(bytes).context("local control request is not UTF-8")
}

/// CLI side of `conduit-daemon.exe send <path>`. Success means the upgraded phone published the
/// Downloads row, not merely that the resident daemon accepted the local path.
pub async fn queue(path: &Path) -> Result<PathBuf> {
    let path = file::validate_outbound(path)?;
    let text = path
        .to_str()
        .context("the path cannot be represented as UTF-8")?;
    let bytes = text.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_REQUEST {
        bail!("path is too long for the local send request");
    }

    let mut client = open_client().await?;

    client.write_u32(bytes.len() as u32).await?;
    client.write_all(bytes).await?;
    let mut reply = Vec::new();
    tokio::time::timeout(
        REMOTE_RESULT_TIMEOUT + Duration::from_secs(5),
        client.read_to_end(&mut reply),
    )
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

/// Asks the already-running daemon to re-read config and apply it in place.
pub async fn reload() -> Result<()> {
    let bytes = RELOAD_COMMAND.as_bytes();
    let mut client = open_client().await?;
    client.write_u32(bytes.len() as u32).await?;
    client.write_all(bytes).await?;
    let mut reply = Vec::new();
    tokio::time::timeout(
        LOCAL_RESULT_TIMEOUT + Duration::from_secs(1),
        client.read_to_end(&mut reply),
    )
    .await
    .context("timed out waiting for Conduit settings to apply")??;
    let reply = String::from_utf8(reply).context("control response is not UTF-8")?;
    if reply == "OK\n" {
        Ok(())
    } else if let Some(error) = reply.strip_prefix("ERR ") {
        bail!("{}", error.trim_end())
    } else {
        bail!("unexpected local control response {reply:?}")
    }
}

async fn open_client() -> Result<NamedPipeClient> {
    let mut last = None;
    for _ in 0..CLIENT_RETRIES {
        match ClientOptions::new().open(PIPE) {
            Ok(pipe) => return Ok(pipe),
            Err(e) if pipe_busy(&e) => {
                last = Some(e);
                tokio::time::sleep(CLIENT_RETRY).await;
            }
            Err(e) => return Err(e).context("opening the Conduit local control pipe"),
        }
    }
    Err(anyhow::anyhow!(
        "Conduit daemon is not accepting local requests{}",
        last.map(|e| format!(": {e}")).unwrap_or_default()
    ))
}

fn pipe_busy(error: &io::Error) -> bool {
    // ERROR_PIPE_BUSY. Windows maps it inconsistently across Rust versions, so preserve the raw
    // value rather than relying only on ErrorKind::WouldBlock.
    error.kind() == io::ErrorKind::WouldBlock || error.raw_os_error() == Some(231)
}
