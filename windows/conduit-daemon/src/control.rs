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
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{info, warn};

use crate::{file, pairing};

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
const PAIR_JOIN_COMMAND: &str = "\0pair-join\0";
const LINK_PAUSE_COMMAND: &str = "\0link-disconnect\0";
const LINK_RESUME_COMMAND: &str = "\0link-connect\0";
const PEER_CONNECT_COMMAND: &str = "\0peer-connect\0";
const PEER_DISCONNECT_COMMAND: &str = "\0peer-disconnect\0";
const PEER_SELECT_COMMAND: &str = "\0peer-select\0";
const PEER_FORGET_COMMAND: &str = "\0peer-forget\0";
const SEND_TO_COMMAND: &str = "\0send-to\0";
const SHARE_URL_COMMAND: &str = "\0share-url\0";
const TRANSFER_CANCEL_COMMAND: &str = "\0transfer-cancel\0";
const NOTIFICATIONS_CLEAR_COMMAND: &str = "\0notifications-clear\0";
const NOTIFICATIONS_PRUNE_COMMAND: &str = "\0notifications-prune\0";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransferProgress {
    pub transferred: u64,
    pub total: u64,
}

pub struct SendRequest {
    pub id: Vec<u8>,
    pub owner: String,
    pub path: PathBuf,
    pub completion: Option<oneshot::Sender<std::result::Result<(), String>>>,
    pub progress: tokio::sync::watch::Sender<TransferProgress>,
}

impl SendRequest {
    pub fn finish(&mut self, result: std::result::Result<(), String>) {
        if let Some(completion) = self.completion.take() {
            let _ = completion.send(result);
        }
    }
}

pub struct ReloadRequest {
    pub completion: oneshot::Sender<std::result::Result<(), String>>,
}

/// Trust, link and per-peer commands, all of which only the main event loop may execute: they
/// rewrite `paired-peers.tsv` or change which sessions exist.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PairAction {
    Start,
    Cancel,
    /// `None` forgets every paired device; `Some(id)` forgets exactly one.
    Forget(Option<String>),
    /// Joins another device's pairing code through its temporary Relay rendezvous.
    Join(String),
    Disconnect,
    Connect,
    PeerConnect(String),
    PeerDisconnect(String),
    PeerSelect(String),
    PeerForget(String),
}

pub struct PairRequest {
    pub action: PairAction,
    pub completion: oneshot::Sender<std::result::Result<(), String>>,
}

/// A user-shared web page for one peer (or the selected peer when `to` is unset).
#[derive(Clone, Debug)]
pub struct SharedUrlRequest {
    pub to: Option<String>,
    pub url: String,
    pub title: String,
}

/// Serves local requests forever. The pipe remains backward compatible with the original
/// path-only sender: only a NUL-prefixed payload is interpreted as a daemon command.
pub async fn serve(
    send_tx: mpsc::Sender<SendRequest>,
    reload_tx: mpsc::Sender<ReloadRequest>,
    pair_tx: mpsc::Sender<PairRequest>,
    share_tx: broadcast::Sender<SharedUrlRequest>,
    transfers: std::sync::Arc<crate::transfer::Transfers>,
    history: std::sync::Arc<crate::notification_history::History>,
    dir: PathBuf,
) -> Result<()> {
    let clients = std::sync::Arc::new(tokio::sync::Semaphore::new(64));
    loop {
        let permit = clients
            .clone()
            .acquire_owned()
            .await
            .context("local client gate stopped")?;
        let pipe = ServerOptions::new()
            .reject_remote_clients(true)
            .create(PIPE)
            .context("creating the Conduit local control pipe")?;
        pipe.connect()
            .await
            .context("accepting a Conduit control client")?;
        let send_tx = send_tx.clone();
        let reload_tx = reload_tx.clone();
        let pair_tx = pair_tx.clone();
        let share_tx = share_tx.clone();
        let transfers = transfers.clone();
        let history = history.clone();
        let dir = dir.clone();
        tokio::spawn(async move {
            let _permit = permit;
            if let Err(e) = handle(
                pipe, send_tx, reload_tx, pair_tx, share_tx, transfers, history, dir,
            )
            .await
            {
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
    share_tx: broadcast::Sender<SharedUrlRequest>,
    transfers: std::sync::Arc<crate::transfer::Transfers>,
    history: std::sync::Arc<crate::notification_history::History>,
    dir: PathBuf,
) -> Result<()> {
    let request = tokio::time::timeout(LOCAL_RESULT_TIMEOUT, read_request(&mut pipe))
        .await
        .context("local client did not complete its bounded request")??;
    if request == NOTIFICATIONS_CLEAR_COMMAND || request == NOTIFICATIONS_PRUNE_COMMAND {
        let result = if request == NOTIFICATIONS_CLEAR_COMMAND {
            history.clear()
        } else {
            history.prune().map(|_| ())
        };
        let response = match result {
            Ok(()) => "OK\n".to_owned(),
            Err(error) => format!("ERR {error:#}\n"),
        };
        tokio::time::timeout(LOCAL_RESULT_TIMEOUT, pipe.write_all(response.as_bytes())).await??;
        let _ = pipe.shutdown().await;
        return Ok(());
    }
    if let Some(id) = request.strip_prefix(TRANSFER_CANCEL_COMMAND) {
        let result = crate::transfer::parse_id(id).and_then(|id| transfers.cancel(&id));
        let reply = match result {
            Ok(()) => "OK\n".into(),
            Err(e) => format!("ERR {e:#}\n"),
        };
        pipe.write_all(reply.as_bytes()).await?;
        let _ = pipe.shutdown().await;
        return Ok(());
    }
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
        reply(&mut pipe, outcome).await;
        return Ok(());
    }
    if let Some(rest) = request.strip_prefix(SHARE_URL_COMMAND) {
        let outcome: Result<()> = async {
            let mut fields = rest.splitn(3, '\t');
            let to = fields.next().unwrap_or_default();
            let url = fields.next().context("shared URL is missing")?;
            let title = fields.next().unwrap_or_default();
            let scheme = url.to_ascii_lowercase();
            if !scheme.starts_with("http://") && !scheme.starts_with("https://") {
                bail!("only http and https links can be shared");
            }
            share_tx
                .send(SharedUrlRequest {
                    to: (!to.is_empty()).then(|| to.to_owned()),
                    url: url.to_owned(),
                    title: title.to_owned(),
                })
                .map_err(|_| anyhow::anyhow!("no live Conduit session is available"))?;
            Ok(())
        }
        .await;
        reply(&mut pipe, outcome).await;
        return Ok(());
    }

    if let Some(action) = trust_action(&request) {
        let outcome = run_trust_action(pair_tx, action).await;
        reply(&mut pipe, outcome).await;
        return Ok(());
    }

    let (requested_peer, path) = match request.strip_prefix(SEND_TO_COMMAND) {
        Some(rest) => match rest.split_once('\0') {
            Some((to, path)) if !to.is_empty() && !path.is_empty() => {
                (Some(to.to_owned()), path.to_owned())
            }
            _ => {
                reply(
                    &mut pipe,
                    Err(anyhow::anyhow!("send --to needs a device id and a path")),
                )
                .await;
                return Ok(());
            }
        },
        None if request.starts_with('\0') => {
            reply(&mut pipe, Err(anyhow::anyhow!("unknown local command"))).await;
            return Ok(());
        }
        None => (None, request),
    };

    let selected = pairing::selected_peer(&dir);
    let phone = pairing::peer_id(&dir);
    let outcome = send_file(
        &mut pipe,
        Path::new(&path),
        requested_peer.as_deref(),
        selected.as_deref(),
        phone.as_deref(),
        send_tx,
        transfers,
    )
    .await;
    match outcome {
        Ok(path) => {
            let _ = tokio::time::timeout(Duration::from_secs(2), pipe.write_all(b"OK\n")).await;
            info!(path = %path.display(), "peer confirmed file publication");
        }
        Err(e) => {
            let reply = format!("ERR {e:#}\n");
            let _ = tokio::time::timeout(Duration::from_secs(2), pipe.write_all(reply.as_bytes()))
                .await;
        }
    }
    let _ = pipe.shutdown().await;
    Ok(())
}

/// Parses one trust/link/per-peer command body. Commands take an optional device id suffix.
fn trust_action(request: &str) -> Option<PairAction> {
    let id = |rest: &str| rest.trim().to_owned();
    match request {
        PAIR_START_COMMAND => Some(PairAction::Start),
        PAIR_CANCEL_COMMAND => Some(PairAction::Cancel),
        LINK_PAUSE_COMMAND => Some(PairAction::Disconnect),
        LINK_RESUME_COMMAND => Some(PairAction::Connect),
        _ => {
            if let Some(rest) = request.strip_prefix(PAIR_FORGET_COMMAND) {
                let id = id(rest);
                return Some(PairAction::Forget((!id.is_empty()).then_some(id)));
            }
            if let Some(rest) = request.strip_prefix(PAIR_JOIN_COMMAND) {
                return Some(PairAction::Join(id(rest)));
            }
            if let Some(rest) = request.strip_prefix(PEER_CONNECT_COMMAND) {
                return Some(PairAction::PeerConnect(id(rest)));
            }
            if let Some(rest) = request.strip_prefix(PEER_DISCONNECT_COMMAND) {
                return Some(PairAction::PeerDisconnect(id(rest)));
            }
            if let Some(rest) = request.strip_prefix(PEER_SELECT_COMMAND) {
                return Some(PairAction::PeerSelect(id(rest)));
            }
            if let Some(rest) = request.strip_prefix(PEER_FORGET_COMMAND) {
                return Some(PairAction::PeerForget(id(rest)));
            }
            None
        }
    }
}

async fn run_trust_action(pair_tx: mpsc::Sender<PairRequest>, action: PairAction) -> Result<()> {
    let (completion, done) = oneshot::channel();
    pair_tx
        .try_send(PairRequest { action, completion })
        .map_err(|e| anyhow::anyhow!("pairing control queue is unavailable: {e}"))?;
    match tokio::time::timeout(LOCAL_RESULT_TIMEOUT, done).await {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(message))) => bail!("{message}"),
        Ok(Err(_)) => bail!("daemon dropped the pairing request"),
        Err(_) => bail!("timed out updating pairing"),
    }
}

async fn reply(pipe: &mut NamedPipeServer, outcome: Result<()>) {
    let body = match outcome {
        Ok(()) => "OK\n".to_owned(),
        Err(e) => format!("ERR {e:#}\n"),
    };
    let _ = pipe.write_all(body.as_bytes()).await;
    let _ = pipe.shutdown().await;
}

#[allow(clippy::too_many_arguments)]
async fn send_file(
    pipe: &mut NamedPipeServer,
    path: &Path,
    requested: Option<&str>,
    selected: Option<&str>,
    phone: Option<&str>,
    send_tx: mpsc::Sender<SendRequest>,
    transfers: std::sync::Arc<crate::transfer::Transfers>,
) -> Result<PathBuf> {
    let mut sessions = transfers.session_changes();
    let expected_owner = transfers.owner_for(requested, selected, phone)?;
    let path = tokio::task::spawn_blocking({
        let path = path.to_path_buf();
        move || file::validate_outbound(&path)
    })
    .await??;
    let total = tokio::fs::metadata(&path).await?.len();
    let (id, owner) = transfers.enqueue(&expected_owner, &path, total)?;
    let (completion, mut done) = oneshot::channel();
    let (progress, mut progress_rx) = tokio::sync::watch::channel(TransferProgress {
        transferred: 0,
        total,
    });
    if let Err(e) = send_tx.try_send(SendRequest {
        id: id.clone(),
        owner: owner.clone(),
        path: path.clone(),
        completion: Some(completion),
        progress,
    }) {
        transfers.update(&owner, &id, 0, "failed", "Outbound queue is full");
        bail!("outbound file queue is unavailable: {e}");
    }

    let deadline = tokio::time::Instant::now() + REMOTE_RESULT_TIMEOUT;
    let mut progress_open = true;
    let mut client_open = true;
    loop {
        tokio::select! {
            update = progress_rx.changed(), if progress_open => {
                match update {
                    Ok(()) if client_open => {
                        let update = *progress_rx.borrow_and_update();
                        let line = format!(
                            "PROGRESS {} {}\n",
                            update.transferred, update.total
                        );
                        if !matches!(tokio::time::timeout(Duration::from_secs(2),
                            pipe.write_all(line.as_bytes())).await, Ok(Ok(()))) {
                            // Explorer can disappear without cancelling a transfer already accepted by
                            // the daemon. Keep the oneshot alive so the transfer remains transactional.
                            client_open = false;
                        }
                    }
                    Ok(()) => { progress_rx.borrow_and_update(); }
                    Err(_) => progress_open = false,
                }
            }
            result = &mut done => {
                break match result {
                    Ok(Ok(())) => Ok(path),
                    Ok(Err(message)) => bail!("peer did not publish the file: {message}"),
                    Err(_) => bail!("the Conduit session ended before the peer confirmed the file"),
                };
            }
            change = sessions.changed() => {
                if change.is_err() || !sessions.borrow_and_update().contains(&owner) {
                    // A queued request may still be in the global bounded channel when its
                    // session ends. Its Explorer/CLI client must not wait for a reconnect.
                    if let Ok(result) = done.try_recv() {
                        break result.map(|()| path).map_err(anyhow::Error::msg);
                    }
                    bail!("the authenticated session ended before confirming this file");
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                bail!("timed out waiting for the peer to publish the file");
            }
        }
    }
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

/// CLI side of `conduit-daemon.exe send [--to <id>] <path>`. Success means the peer published the
/// Downloads row, not merely that the resident daemon accepted the local path. The addressed form
/// never falls back to another device.
pub async fn queue_to_progress<F>(to: Option<&str>, path: &Path, on_progress: F) -> Result<PathBuf>
where
    F: FnMut(u64, u64),
{
    let path = file::validate_outbound(path)?;
    let text = path
        .to_str()
        .context("the path cannot be represented as UTF-8")?;
    let request = match to {
        Some(id) => format!("{SEND_TO_COMMAND}{id}\0{text}"),
        None => text.to_owned(),
    };
    let bytes = request.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_REQUEST {
        bail!("path is too long for the local send request");
    }

    let mut client = open_client().await?;
    client.write_u32(bytes.len() as u32).await?;
    client.write_all(bytes).await?;

    let mut lines = BufReader::new(client).lines();
    let mut on_progress = on_progress;
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

pub async fn notifications(clear: bool) -> Result<()> {
    simple_command(
        if clear {
            NOTIFICATIONS_CLEAR_COMMAND
        } else {
            NOTIFICATIONS_PRUNE_COMMAND
        },
        "notification history update",
    )
    .await
}

pub async fn cancel(id: &str) -> Result<()> {
    crate::transfer::parse_id(id)?;
    simple_command(
        &format!("{TRANSFER_CANCEL_COMMAND}{id}"),
        "transfer cancellation",
    )
    .await
}

pub async fn pair(action: PairAction) -> Result<()> {
    let what = "Conduit pairing state to update";
    match action {
        PairAction::Start => simple_command(PAIR_START_COMMAND, what).await,
        PairAction::Cancel => simple_command(PAIR_CANCEL_COMMAND, what).await,
        PairAction::Forget(id) => match id {
            Some(id) => simple_command(&format!("{PAIR_FORGET_COMMAND}{id}"), what).await,
            None => simple_command(PAIR_FORGET_COMMAND, what).await,
        },
        PairAction::Join(code) => simple_command(&format!("{PAIR_JOIN_COMMAND}{code}"), what).await,
        PairAction::Disconnect => simple_command(LINK_PAUSE_COMMAND, what).await,
        PairAction::Connect => simple_command(LINK_RESUME_COMMAND, what).await,
        PairAction::PeerConnect(id) => {
            simple_command(&format!("{PEER_CONNECT_COMMAND}{id}"), what).await
        }
        PairAction::PeerDisconnect(id) => {
            simple_command(&format!("{PEER_DISCONNECT_COMMAND}{id}"), what).await
        }
        PairAction::PeerSelect(id) => {
            simple_command(&format!("{PEER_SELECT_COMMAND}{id}"), what).await
        }
        PairAction::PeerForget(id) => {
            simple_command(&format!("{PEER_FORGET_COMMAND}{id}"), what).await
        }
    }
}

/// `share-url [--to <id>] <url> [title]`. The daemon puts it on the addressed live session, so a
/// shared page can never be delivered to whichever peer happened to connect first.
pub async fn share_url(to: Option<&str>, url: &str, title: &str) -> Result<()> {
    let command = format!(
        "{SHARE_URL_COMMAND}{}\t{}\t{}",
        to.unwrap_or_default(),
        url.replace(['\t', '\n', '\r'], " "),
        title.replace(['\t', '\n', '\r'], " "),
    );
    simple_command(&command, "shared URL delivery").await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_commands_round_trip_with_and_without_a_device_id() {
        assert_eq!(trust_action(PAIR_START_COMMAND), Some(PairAction::Start));
        assert_eq!(trust_action(LINK_RESUME_COMMAND), Some(PairAction::Connect));
        assert_eq!(
            trust_action(PAIR_FORGET_COMMAND),
            Some(PairAction::Forget(None))
        );
        assert_eq!(
            trust_action(&format!("{PAIR_FORGET_COMMAND}studio")),
            Some(PairAction::Forget(Some("studio".into())))
        );
        assert_eq!(
            trust_action(&format!("{PAIR_JOIN_COMMAND}12 34 56")).map(|action| match action {
                PairAction::Join(code) => code,
                other => panic!("unexpected {other:?}"),
            }),
            Some("12 34 56".into())
        );
        for (command, want) in [
            (PEER_CONNECT_COMMAND, "connect"),
            (PEER_DISCONNECT_COMMAND, "disconnect"),
            (PEER_SELECT_COMMAND, "select"),
            (PEER_FORGET_COMMAND, "forget"),
        ] {
            let action = trust_action(&format!("{command}studio")).unwrap();
            let (name, id) = match action {
                PairAction::PeerConnect(id) => ("connect", id),
                PairAction::PeerDisconnect(id) => ("disconnect", id),
                PairAction::PeerSelect(id) => ("select", id),
                PairAction::PeerForget(id) => ("forget", id),
                other => panic!("unexpected {other:?}"),
            };
            assert_eq!((name, id.as_str()), (want, "studio"));
        }
        // A plain local path is still a legacy path-only send, never a command.
        assert_eq!(trust_action("C:\\Users\\me\\a.bin"), None);
        assert_eq!(trust_action(TRANSFER_CANCEL_COMMAND), None);
    }

    #[test]
    fn send_to_and_share_url_requests_are_parsed_from_their_command_bodies() {
        let body = format!("{SEND_TO_COMMAND}studio\0C:\\a.bin");
        let rest = body.strip_prefix(SEND_TO_COMMAND).unwrap();
        let (to, path) = rest.split_once('\0').unwrap();
        assert_eq!((to, path), ("studio", "C:\\a.bin"));
        let body = format!("{SHARE_URL_COMMAND}\t\thttps://example.com");
        let rest = body.strip_prefix(SHARE_URL_COMMAND).unwrap();
        let mut fields = rest.splitn(3, '\t');
        assert_eq!(fields.next(), Some(""));
        assert_eq!(fields.next(), Some(""));
        assert_eq!(fields.next(), Some("https://example.com"));
    }
}
