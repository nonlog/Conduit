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
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
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
const PAIR_START_COMMAND: &str = "\0pair-start\0";
const PAIR_CANCEL_COMMAND: &str = "\0pair-cancel\0";
const PAIR_FORGET_COMMAND: &str = "\0pair-forget\0";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransferProgress {
    pub transferred: u64,
    pub total: u64,
}

pub struct SendRequest {
    pub path: PathBuf,
    pub completion: oneshot::Sender<std::result::Result<(), String>>,
    pub progress: mpsc::UnboundedSender<TransferProgress>,
}

pub struct ReloadRequest {
    pub completion: oneshot::Sender<std::result::Result<(), String>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairAction {
    Start,
    Cancel,
    Forget,
}

pub struct PairRequest {
    pub action: PairAction,
    pub completion: oneshot::Sender<std::result::Result<(), String>>,
}

/// Serves local requests forever. The pipe remains backward compatible with the original
/// path-only sender: only a NUL-prefixed payload is interpreted as a daemon command.
pub async fn serve(
    send_tx: mpsc::Sender<SendRequest>,
    reload_tx: mpsc::Sender<ReloadRequest>,
    pair_tx: mpsc::Sender<PairRequest>,
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
        let pair_tx = pair_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(pipe, send_tx, reload_tx, pair_tx).await {
                warn!(error = %e, "local control request failed");
            }
        });
    }
}

async fn handle(
    mut pipe: NamedPipeServer,
    send_tx: mpsc::Sender<SendRequest>,
    reload_tx: mpsc::Sender<ReloadRequest>,
    pair_tx: mpsc::Sender<PairRequest>,
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

    let pair_action = match request.as_str() {
        PAIR_START_COMMAND => Some(PairAction::Start),
        PAIR_CANCEL_COMMAND => Some(PairAction::Cancel),
        PAIR_FORGET_COMMAND => Some(PairAction::Forget),
        _ => None,
    };
    if let Some(action) = pair_action {
        let outcome: Result<()> = async {
            let (completion, done) = oneshot::channel();
            pair_tx
                .try_send(PairRequest { action, completion })
                .map_err(|e| anyhow::anyhow!("pairing control queue is unavailable: {e}"))?;
            match tokio::time::timeout(LOCAL_RESULT_TIMEOUT, done).await {
                Ok(Ok(Ok(()))) => Ok(()),
                Ok(Ok(Err(message))) => bail!("could not update pairing: {message}"),
                Ok(Err(_)) => bail!("daemon dropped the pairing request"),
                Err(_) => bail!("timed out updating pairing"),
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
        let (completion, mut done) = oneshot::channel();
        let (progress, mut progress_rx) = mpsc::unbounded_channel();
        send_tx
            .try_send(SendRequest {
                path: path.clone(),
                completion,
                progress,
            })
            .map_err(|e| anyhow::anyhow!("outbound file queue is unavailable: {e}"))?;

        let deadline = tokio::time::Instant::now() + REMOTE_RESULT_TIMEOUT;
        let mut progress_open = true;
        let mut client_open = true;
        loop {
            tokio::select! {
                update = progress_rx.recv(), if progress_open => {
                    match update {
                        Some(update) if client_open => {
                            let line = format!(
                                "PROGRESS {} {}\n",
                                update.transferred, update.total
                            );
                            if pipe.write_all(line.as_bytes()).await.is_err() {
                                // Explorer can disappear without cancelling a transfer already accepted by
                                // the daemon. Keep the oneshot alive so the transfer remains transactional.
                                client_open = false;
                            }
                        }
                        Some(_) => {}
                        None => progress_open = false,
                    }
                }
                result = &mut done => {
                    break match result {
                        Ok(Ok(())) => Ok(path),
                        Ok(Err(message)) => bail!("phone did not publish the file: {message}"),
                        Err(_) => bail!("the Conduit session ended before the phone confirmed the file"),
                    };
                }
                _ = tokio::time::sleep_until(deadline) => {
                    bail!("timed out waiting for the phone to publish the file");
                }
            }
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
    queue_with_progress(path, |_, _| {}).await
}

pub async fn queue_with_progress<F>(path: &Path, mut on_progress: F) -> Result<PathBuf>
where
    F: FnMut(u64, u64),
{
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

    let mut lines = BufReader::new(client).lines();
    loop {
        let line = tokio::time::timeout(
            REMOTE_RESULT_TIMEOUT + Duration::from_secs(5),
            lines.next_line(),
        )
        .await
        .context("timed out waiting for the resident Conduit daemon")??
        .context("resident Conduit daemon closed the control pipe without a result")?;

        if line == "OK" {
            return Ok(path);
        }
        if let Some(error) = line.strip_prefix("ERR ") {
            bail!("{}", error.trim_end());
        }
        if let Some(rest) = line.strip_prefix("PROGRESS ") {
            let mut fields = rest.split_whitespace();
            let transferred = fields.next().and_then(|value| value.parse::<u64>().ok());
            let total = fields.next().and_then(|value| value.parse::<u64>().ok());
            if let (Some(transferred), Some(total)) = (transferred, total) {
                on_progress(transferred, total);
                continue;
            }
        }
        bail!("unexpected local control response {line:?}");
    }
}

/// Asks the already-running daemon to re-read config and apply it in place.
pub async fn reload() -> Result<()> {
    simple_command(RELOAD_COMMAND, "Conduit settings to apply").await
}

pub async fn pair(action: PairAction) -> Result<()> {
    let command = match action {
        PairAction::Start => PAIR_START_COMMAND,
        PairAction::Cancel => PAIR_CANCEL_COMMAND,
        PairAction::Forget => PAIR_FORGET_COMMAND,
    };
    simple_command(command, "Conduit pairing state to update").await
}

async fn simple_command(command: &str, what: &str) -> Result<()> {
    let bytes = command.as_bytes();
    let mut client = open_client().await?;
    client.write_u32(bytes.len() as u32).await?;
    client.write_all(bytes).await?;
    let mut reply = Vec::new();
    tokio::time::timeout(
        LOCAL_RESULT_TIMEOUT + Duration::from_secs(1),
        client.read_to_end(&mut reply),
    )
    .await
    .with_context(|| format!("timed out waiting for {what}"))??;
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
