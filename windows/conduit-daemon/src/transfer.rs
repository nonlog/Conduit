//! Session-owned file state and bounded disk workers. No phone polling.
use crate::{
    control, file,
    wire::{self, pb},
};
use anyhow::{bail, Context, Result};
use prost::Message as _;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, watch, Notify};

/// The session alone encrypts frames; this worker writes ciphertext in exactly that order.
/// One data slot and three control slots keep RX live even when the socket write is blocked.
pub struct Writer {
    tx: mpsc::Sender<Vec<u8>>,
    frame: Vec<u8>,
    queued: u64,
    written: watch::Receiver<u64>,
    pub task: tokio::task::JoinHandle<Result<()>>,
}
impl Writer {
    pub fn start<W: tokio::io::AsyncWrite + Unpin + Send + 'static>(mut socket: W) -> Self {
        use tokio::io::AsyncWriteExt as _;
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(4);
        let (written_tx, written) = watch::channel(0u64);
        let task = tokio::spawn(async move {
            let mut completed = 0;
            while let Some(frame) = rx.recv().await {
                // Never race/timeout an individual frame write.
                socket.write_all(&frame).await?;
                socket.flush().await?;
                completed += 1;
                written_tx.send_replace(completed);
            }
            Ok(())
        });
        Self {
            tx,
            frame: Vec::new(),
            queued: 0,
            written,
            task,
        }
    }
    pub fn data_ready(&self) -> bool {
        self.tx.capacity() == 4
    }
    pub fn control_ready(&self) -> bool {
        self.tx.capacity() > 0
    }
    pub fn slots(&self) -> mpsc::Sender<Vec<u8>> {
        self.tx.clone()
    }
    pub async fn drain(&self) -> Result<()> {
        let mut written = self.written.clone();
        written
            .wait_for(|count| *count >= self.queued)
            .await
            .context("socket writer stopped before control frame was delivered")?;
        Ok(())
    }
}
impl tokio::io::AsyncWrite for Writer {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        if self.frame.len().saturating_add(bytes.len()) > wire::MAX_FRAME + 4 {
            return std::task::Poll::Ready(Err(std::io::Error::other("oversized encrypted frame")));
        }
        self.frame.extend_from_slice(bytes);
        std::task::Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if self.frame.is_empty() {
            return std::task::Poll::Ready(Ok(()));
        }
        let frame = std::mem::take(&mut self.frame);
        let result = self
            .tx
            .try_send(frame)
            .map_err(|_| std::io::Error::other("encrypted control queue exhausted"));
        if result.is_ok() {
            self.queued += 1;
        }
        std::task::Poll::Ready(result)
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.poll_flush(cx)
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub const RESULT_TIMEOUT: Duration = Duration::from_secs(60);
const RECEIVE_TIMEOUT: Duration = Duration::from_secs(120);
const HISTORY: usize = 32;

pub fn new_id() -> Result<Vec<u8>> {
    // snow already uses the OS CSPRNG for identities; no clock/path-derived identifiers.
    Ok(snow::Builder::new(wire::NOISE_XX.parse()?)
        .generate_keypair()?
        .public[..16]
        .to_vec())
}
pub fn hex(id: &[u8]) -> String {
    id.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn parse_id(text: &str) -> Result<Vec<u8>> {
    if text.is_empty()
        || text.len() > 128
        || text.len() % 2 != 0
        || !text.bytes().all(|c| c.is_ascii_hexdigit())
    {
        bail!("invalid transfer id");
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(Into::into))
        .collect()
}

#[derive(Clone, Debug)]
struct Row {
    id: Vec<u8>,
    owner: String,
    name: String,
    direction: &'static str,
    bytes: u64,
    total: u64,
    state: &'static str,
    detail: String,
    cancel: bool,
}
fn terminal(state: &str) -> bool {
    matches!(state, "completed" | "failed" | "cancelled")
}
#[derive(Default)]
struct State {
    live: Option<(String, bool)>,
    rows: VecDeque<Row>,
}

// transfers.tsv is also the desktop's recent-transfer history. Its rows are
// newest-first, while the in-memory queue is oldest-first. Rebuild the bounded
// queue before the snapshot writer starts, otherwise restarting the daemon
// replaces perfectly good persisted history with an empty file.
fn restore_snapshot(body: &str) -> VecDeque<Row> {
    let mut newest_first: Vec<Row> = Vec::new();
    for line in body.lines() {
        if newest_first.len() == HISTORY {
            break;
        }
        let fields: Vec<_> = line.splitn(7, '\t').collect();
        if fields.len() != 7 {
            continue;
        }
        let Ok(id) = parse_id(fields[0]) else {
            continue;
        };
        if id.len() < 16 || id.len() > 64 || newest_first.iter().any(|row| row.id == id) {
            continue;
        }
        let direction = match fields[1] {
            "inbound" => "inbound",
            "outbound" => "outbound",
            _ => continue,
        };
        let (Ok(bytes), Ok(total)) = (fields[2].parse::<u64>(), fields[3].parse::<u64>()) else {
            continue;
        };
        if bytes > total {
            continue;
        }
        // A previous process's socket, queued work and cancellation lease no
        // longer exist. Never resurrect them as actionable tasks in the UI.
        let (state, detail) = match fields[4] {
            "completed" => ("completed", fields[6].chars().take(512).collect()),
            "failed" => ("failed", fields[6].chars().take(512).collect()),
            "cancelled" => ("cancelled", fields[6].chars().take(512).collect()),
            "queued" | "transferring" | "waiting confirmation" => (
                "failed",
                "Transfer interrupted by desktop restart".to_owned(),
            ),
            _ => continue,
        };
        newest_first.push(Row {
            id,
            owner: String::new(), // old rows must not belong to a new session
            name: fields[5].chars().take(256).collect(),
            direction,
            bytes,
            total,
            state,
            detail,
            cancel: false,
        });
    }
    newest_first.reverse();
    newest_first.into()
}

pub struct Transfers {
    state: Mutex<State>,
    changed: watch::Sender<u64>,
    sessions: watch::Sender<Option<String>>,
    pub wake: Notify,
}
impl Transfers {
    pub fn start(dir: &Path) -> Arc<Self> {
        let path = dir.join("transfers.tsv");
        let rows = match std::fs::read_to_string(&path) {
            Ok(body) => restore_snapshot(&body),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => VecDeque::new(),
            Err(error) => {
                tracing::warn!(%error, "could not restore transfer history");
                VecDeque::new()
            }
        };
        let (changed, mut rx) = watch::channel(0u64);
        let (sessions, _) = watch::channel(None);
        let this = Arc::new(Self {
            state: Mutex::new(State { live: None, rows }),
            changed,
            sessions,
            wake: Notify::new(),
        });
        let state = Arc::downgrade(&this);
        // One coalescing local writer, asleep while idle. The existing UI watcher reads this file.
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return this;
        };
        runtime.spawn(async move {
            loop {
                let body = match state.upgrade() {
                    Some(state) => state.snapshot(),
                    None => break,
                };
                let path = path.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    let temp = path.with_extension("tsv.tmp");
                    std::fs::write(&temp, body)?;
                    std::fs::rename(temp, path)
                })
                .await
                .map(|result| {
                    if let Err(e) = result {
                        tracing::warn!(error = %e, "transfer snapshot failed");
                    }
                });
                tokio::time::sleep(Duration::from_millis(250)).await;
                if rx.changed().await.is_err() {
                    break;
                }
                rx.borrow_and_update();
            }
        });
        this
    }
    fn changed(&self) {
        self.changed.send_modify(|n| *n = n.wrapping_add(1));
    }
    fn snapshot(&self) -> String {
        fn clean(s: &str) -> String {
            s.replace(['\t', '\r', '\n'], " ")
        }
        let state = self.state.lock().unwrap();
        let mut rows: Vec<_> = state.rows.iter().rev().collect();
        rows.sort_by_key(|row| {
            if terminal(row.state) {
                2
            } else if row.state == "queued" {
                1
            } else {
                0
            }
        });
        rows.into_iter()
            .map(|r| {
                format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    hex(&r.id),
                    r.direction,
                    r.bytes,
                    r.total,
                    r.state,
                    clean(&r.name),
                    clean(&r.detail)
                )
            })
            .collect()
    }
    pub fn session(self: &Arc<Self>, capable: bool) -> Result<Lease> {
        let owner = hex(&new_id()?);
        {
            let mut state = self.state.lock().unwrap();
            state.live = Some((owner.clone(), capable));
            self.sessions.send_replace(Some(owner.clone()));
        }
        Ok(Lease {
            transfers: self.clone(),
            owner,
        })
    }
    fn add(s: &mut State, row: Row) -> Result<()> {
        if s.rows.iter().any(|r| r.id == row.id) {
            bail!("transfer id already used");
        }
        if s.rows.len() >= HISTORY {
            let i = s
                .rows
                .iter()
                .position(|r| terminal(r.state))
                .context("transfer queue is full")?;
            s.rows.remove(i);
        }
        s.rows.push_back(row);
        Ok(())
    }
    pub fn owner(&self) -> Result<String> {
        self.state
            .lock()
            .unwrap()
            .live
            .as_ref()
            .map(|(owner, _)| owner.clone())
            .context("no authenticated phone session")
    }
    /// Includes queued and receiver-confirmation work, not only byte streaming.
    pub fn has_active(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.live.as_ref().is_some_and(|(owner, _)| {
            state
                .rows
                .iter()
                .any(|row| &row.owner == owner && !terminal(row.state))
        })
    }
    pub fn session_changes(&self) -> watch::Receiver<Option<String>> {
        self.sessions.subscribe()
    }
    pub fn enqueue(
        &self,
        expected_owner: &str,
        path: &Path,
        total: u64,
    ) -> Result<(Vec<u8>, String)> {
        let mut s = self.state.lock().unwrap();
        let owner = s
            .live
            .as_ref()
            .context("no authenticated phone session")?
            .0
            .clone();
        if owner != expected_owner {
            bail!("phone session changed while opening the file; send again");
        }
        let id = new_id()?;
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        Self::add(
            &mut s,
            Row {
                id: id.clone(),
                owner: owner.clone(),
                name,
                direction: "outbound",
                bytes: 0,
                total,
                state: "queued",
                detail: String::new(),
                cancel: false,
            },
        )?;
        drop(s);
        self.changed();
        Ok((id, owner))
    }
    pub fn incoming(&self, owner: &str, offer: &pb::FileOffer) -> Result<()> {
        if offer.transfer_id.is_empty() || offer.transfer_id.len() > 64 {
            bail!("invalid transfer id");
        }
        let mut s = self.state.lock().unwrap();
        Self::add(
            &mut s,
            Row {
                id: offer.transfer_id.clone(),
                owner: owner.into(),
                name: offer.name.chars().take(256).collect(),
                direction: "inbound",
                bytes: 0,
                total: offer.total_bytes,
                state: "transferring",
                detail: String::new(),
                cancel: false,
            },
        )?;
        drop(s);
        self.changed();
        Ok(())
    }
    pub fn update(&self, owner: &str, id: &[u8], bytes: u64, state: &'static str, detail: &str) {
        let mut s = self.state.lock().unwrap();
        if let Some(r) = s
            .rows
            .iter_mut()
            .find(|r| r.owner == owner && r.id == id && !terminal(r.state))
        {
            r.bytes = bytes.min(r.total);
            r.state = state;
            r.detail = detail.chars().take(512).collect();
        }
        drop(s);
        self.changed();
    }
    pub fn published(&self, owner: &str, id: &[u8], bytes: u64) {
        let mut s = self.state.lock().unwrap();
        // Only the receive worker with proof of publication can settle a disconnect/cancel race.
        if let Some(row) = s
            .rows
            .iter_mut()
            .find(|r| r.owner == owner && r.id == id && r.direction == "inbound")
        {
            row.bytes = bytes.min(row.total);
            row.state = "completed";
            row.detail.clear();
        }
        drop(s);
        self.changed();
    }
    pub fn cancelled(&self, owner: &str, id: &[u8]) -> bool {
        self.state
            .lock()
            .unwrap()
            .rows
            .iter()
            .any(|r| r.owner == owner && r.id == id && r.cancel)
    }
    /// Serialises the queued-to-offered transition against a legacy peer's queued cancellation.
    pub fn claim(&self, owner: &str, id: &[u8], total: u64) -> bool {
        let mut s = self.state.lock().unwrap();
        if s.live.as_ref().is_none_or(|(live, _)| live != owner) {
            return false;
        }
        let Some(row) = s.rows.iter_mut().find(|r| r.owner == owner && r.id == id) else {
            return false;
        };
        if row.cancel || terminal(row.state) {
            return false;
        }
        row.state = "transferring";
        row.total = total;
        drop(s);
        self.changed();
        true
    }
    pub fn cancel(&self, id: &[u8]) -> Result<()> {
        let mut s = self.state.lock().unwrap();
        let (owner, capable) = s.live.clone().context("no authenticated phone session")?;
        let row = s
            .rows
            .iter_mut()
            .find(|r| r.owner == owner && r.id == id)
            .context("transfer is no longer in this session")?;
        if terminal(row.state) {
            return Ok(());
        }
        if !capable && row.state != "queued" {
            bail!("this phone does not support targeted cancellation; update Conduit on the phone");
        }
        row.cancel = true;
        row.detail = "Cancellation requested".into();
        if row.state != "queued" {
            row.state = "waiting confirmation";
        }
        drop(s);
        self.changed();
        self.wake.notify_one();
        Ok(())
    }
}
pub struct Lease {
    pub transfers: Arc<Transfers>,
    pub owner: String,
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut s = self.transfers.state.lock().unwrap();
        if s.live
            .as_ref()
            .is_some_and(|(owner, _)| owner == &self.owner)
        {
            s.live = None;
            self.transfers.sessions.send_replace(None);
        }
        for r in &mut s.rows {
            if r.owner == self.owner && !terminal(r.state) {
                r.state = "failed";
                r.detail = "Session disconnected".into();
            }
        }
        drop(s);
        self.transfers.changed();
    }
}

pub struct Frame {
    pub kind: pb::Kind,
    pub payload: Vec<u8>,
    pub bytes: u64,
    pub total: u64,
}
pub struct Sending {
    pub request: control::SendRequest,
    pub frames: mpsc::Receiver<Result<Frame>>,
    pub sent: u64,
    pub total: u64,
    pub waiting: Option<tokio::time::Instant>,
    pub last_progress: std::time::Instant,
    pub cancel_sent: bool,
    producer: tokio::task::JoinHandle<()>,
}
impl Sending {
    pub fn start(request: control::SendRequest) -> Self {
        let (tx, frames) = mpsc::channel(1);
        let path = request.path.clone();
        let id = request.id.clone();
        let producer = tokio::spawn(async move {
            let result: Result<()> = async {
                let mut source = file::Outbound::open(&path).await?;
                let offer = source.offer(&id);
                let total = offer.total_bytes;
                tx.send(Ok(Frame {
                    kind: pb::Kind::FileOffer,
                    payload: offer.encode_to_vec(),
                    bytes: 0,
                    total,
                }))
                .await
                .ok();
                let mut bytes = 0;
                loop {
                    let permit = tx.reserve().await.context("sender closed")?;
                    let Some(chunk) = source.next_chunk().await? else {
                        break;
                    };
                    bytes += chunk.data.len() as u64;
                    permit.send(Ok(Frame {
                        kind: pb::Kind::FileChunk,
                        payload: chunk.encode_to_vec(),
                        bytes,
                        total,
                    }));
                }
                Ok(())
            }
            .await;
            if let Err(e) = result {
                let _ = tx.send(Err(e)).await;
            }
        });
        Self {
            request,
            frames,
            sent: 0,
            total: 0,
            waiting: None,
            producer,
            last_progress: std::time::Instant::now(),
            cancel_sent: false,
        }
    }
    pub fn stop_producer(&mut self) {
        self.producer.abort();
    }
    pub fn expired(&self, now: tokio::time::Instant) -> bool {
        self.waiting.is_some_and(|d| now >= d)
    }
    pub fn accepts(&self, result: &pb::FileResult) -> bool {
        self.request.id == result.transfer_id
            && (!result.success
                || (self.waiting.is_some() && self.total > 0 && self.sent == self.total))
    }
}
impl Drop for Sending {
    fn drop(&mut self) {
        self.producer.abort();
    }
}

pub struct Received {
    pub result: pb::FileResult,
    pub path: Option<PathBuf>,
}
pub struct Receiving {
    pub id: Vec<u8>,
    pub tx: mpsc::Sender<pb::FileChunk>,
    cancel: Arc<AtomicBool>,
    pub cancel_sent: bool,
    wake: Arc<Notify>,
}
impl Receiving {
    pub fn start(
        offer: pb::FileOffer,
        dir: PathBuf,
        owner: String,
        transfers: Arc<Transfers>,
        results: mpsc::Sender<Received>,
    ) -> Self {
        let (tx, mut rx) = mpsc::channel::<pb::FileChunk>(4);
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let wake = Arc::new(Notify::new());
        let worker_wake = wake.clone();
        let id = offer.transfer_id.clone();
        let worker_id = id.clone();
        tokio::spawn(async move {
            let begin = tokio::task::spawn_blocking(move || {
                let configured = crate::config::Config::load(&dir)?.receive_dir;
                file::Incoming::begin(&offer, &file::downloads(configured.as_deref())?)
                    .map(|file| (file, offer))
            })
            .await;
            let mut path = None;
            let mut bytes = 0;
            let mut name = String::new();
            let mut cancelled = false;
            let outcome: Result<()> = async {
                let (mut file, offer) = begin.context("receive worker stopped")??;
                name = file.name().into();
                let mut progress_deadline = tokio::time::Instant::now() + RECEIVE_TIMEOUT;
                loop {
                    if flag.load(Ordering::Acquire) {
                        cancelled = true;
                        bail!("Cancelled");
                    }
                    let chunk = tokio::select! {
                        chunk = rx.recv() => chunk.context("session disconnected")?,
                        _ = worker_wake.notified() => continue,
                        _ = tokio::time::sleep_until(progress_deadline) => bail!("Receive stalled for 120 seconds"),
                    };
                    let write_flag = flag.clone();
                    let step = tokio::task::spawn_blocking(move || {
                        let result = file.push_cancellable(&chunk, &write_flag);
                        (file, result)
                    })
                    .await
                    .context("receive write worker stopped")?;
                    file = step.0;
                    // If publication won the race, completion takes priority over cancellation.
                    match step.1 {
                        Ok(Some(published)) => {
                            bytes = offer.total_bytes;
                            path = Some(published);
                            return Ok(());
                        }
                        Ok(None) => {
                            bytes = file.written();
                            progress_deadline = tokio::time::Instant::now() + RECEIVE_TIMEOUT;
                            transfers.update(&owner, &offer.transfer_id, bytes, "transferring", "");
                        }
                        Err(e) => {
                            cancelled = flag_cancelled(&flag);
                            return Err(e);
                        }
                    }
                }
            }
            .await;
            let detail = outcome
                .as_ref()
                .err()
                .map(|e| format!("{e:#}"))
                .unwrap_or_default();
            let success = outcome.is_ok();
            let state = if success {
                "completed"
            } else if cancelled {
                "cancelled"
            } else {
                "failed"
            };
            // id is held separately because a refused offer has no Incoming instance.
            if success {
                transfers.published(&owner, &worker_id, bytes);
            } else {
                transfers.update(&owner, &worker_id, bytes, state, &detail);
            }
            let _ = results
                .send(Received {
                    result: pb::FileResult {
                        transfer_id: worker_id,
                        success,
                        error: detail,
                        name,
                        cancelled,
                    },
                    path,
                })
                .await;
        });
        Self {
            id,
            tx,
            cancel,
            wake,
            cancel_sent: false,
        }
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
        self.wake.notify_one();
    }
}
fn flag_cancelled(flag: &AtomicBool) -> bool {
    flag.load(Ordering::Acquire)
}
impl Drop for Receiving {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Arc<Transfers> {
        let (changed, _) = watch::channel(0);
        let (sessions, _) = watch::channel(None);
        Arc::new(Transfers {
            state: Mutex::new(State::default()),
            changed,
            sessions,
            wake: Notify::new(),
        })
    }
    fn incoming_offer(id: u8) -> pb::FileOffer {
        pb::FileOffer {
            name: "file.bin".into(),
            mime: "application/octet-stream".into(),
            total_bytes: 4,
            chunk_size: 4,
            chunk_count: 1,
            transfer_id: vec![id; 16],
            timestamp_ms: 0,
        }
    }

    #[test]
    fn completed_history_survives_reloading_and_a_new_session() -> Result<()> {
        let first = "11".repeat(16);
        let second = "22".repeat(16);
        let body = format!(
            "{second}\toutbound\t0\t10\tcancelled\tphoto.jpg\tUser cancelled\n\
             {first}\tinbound\t4\t4\tcompleted\tfile.bin\t\n"
        );
        let restored = restore_snapshot(&body);
        assert_eq!(restored.len(), 2);
        let registry = registry();
        registry.state.lock().unwrap().rows = restored;
        assert_eq!(registry.snapshot(), body);
        let lease = registry.session(true)?;
        let (id, _) = registry.enqueue(&lease.owner, Path::new("new.bin"), 10)?;
        let updated = registry.snapshot();
        assert!(updated.contains(&first));
        assert!(updated.contains(&second));
        assert!(updated.contains(&hex(&id)));
        Ok(())
    }

    #[test]
    fn restart_never_reactivates_interrupted_or_malformed_transfers() {
        let first = "33".repeat(16);
        let second = "44".repeat(16);
        let body = format!(
            "{first}\toutbound\t5\t10\ttransferring\tin-flight.bin\t\n\
             {second}\tinbound\t0\t10\twaiting confirmation\twaiting.bin\t\n\
             broken\toutbound\t3\t2\tcompleted\tbad.bin\t\n"
        );
        let rows = restore_snapshot(&body);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| terminal(row.state)));
        assert!(rows
            .iter()
            .all(|row| row.detail == "Transfer interrupted by desktop restart"));
        assert!(rows.iter().all(|row| row.owner.is_empty() && !row.cancel));
        let registry = registry();
        registry.state.lock().unwrap().rows = rows;
        assert!(!registry.has_active());
        assert!(!registry.snapshot().contains("\ttransferring\t"));
        assert!(!registry.snapshot().contains("\twaiting confirmation\t"));
    }

    #[tokio::test]
    async fn daemon_start_does_not_clear_existing_disk_history() -> Result<()> {
        let name = format!(
            "conduit-history-recovery-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        );
        let dir = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("transfers.tsv");
        let original = format!(
            "{}\tinbound\t10\t10\tcompleted\timportant.bin\t\n",
            "55".repeat(16)
        );
        std::fs::write(&path, &original)?;
        let registry = Transfers::start(&dir);
        assert_eq!(registry.snapshot(), original);
        tokio::time::sleep(Duration::from_millis(350)).await;
        assert_eq!(std::fs::read_to_string(&path)?, original);
        let lease = registry.session(true)?;
        registry.enqueue(&lease.owner, Path::new("new.bin"), 8)?;
        tokio::time::sleep(Duration::from_millis(350)).await;
        let latest = std::fs::read_to_string(&path)?;
        assert!(latest.contains("important.bin"));
        assert!(latest.contains("new.bin"));
        drop(lease);
        drop(registry);
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }

    #[tokio::test]
    async fn a_blocked_socket_never_blocks_enqueuing_or_reorders_a_frame() -> Result<()> {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let (socket, mut peer) = tokio::io::duplex(1);
        let mut writer = Writer::start(socket);
        let first = b"first whole frame";
        let second = b"second whole frame";
        // The peer has not read one byte. Preparing complete encrypted frames must still finish.
        writer.write_all(first).await?;
        writer.flush().await?;
        writer.write_all(second).await?;
        writer.flush().await?;
        let mut bytes = vec![0; first.len() + second.len()];
        peer.read_exact(&mut bytes).await?;
        assert_eq!(bytes, [first.as_slice(), second.as_slice()].concat());
        Ok(())
    }

    #[test]
    fn cancel_ids_are_bounded_and_strictly_hexadecimal() {
        assert_eq!(parse_id("00aAFF").unwrap(), vec![0, 170, 255]);
        for id in ["", "a", "xx", "00\0", "ä0"] {
            assert!(parse_id(id).is_err());
        }
        assert!(parse_id(&"a".repeat(130)).is_err());
    }

    #[test]
    fn cancellation_is_targeted_and_legacy_claim_is_atomic() -> Result<()> {
        let registry = registry();
        let lease = registry.session(false)?;
        let (first, owner) = registry.enqueue(&lease.owner, Path::new("one.bin"), 4)?;
        let (second, _) = registry.enqueue(&lease.owner, Path::new("two.bin"), 4)?;
        registry.cancel(&first)?;
        assert!(registry.cancelled(&owner, &first));
        assert!(!registry.cancelled(&owner, &second));
        assert!(!registry.claim(&owner, &first, 4));
        assert!(registry.claim(&owner, &second, 4));
        assert!(registry
            .cancel(&second)
            .unwrap_err()
            .to_string()
            .contains("does not support"));
        assert!(registry.cancel(&[99; 16]).is_err());
        drop(lease);
        assert!(registry.enqueue(&owner, Path::new("later.bin"), 4).is_err());
        Ok(())
    }

    #[test]
    fn completed_publication_wins_a_cancel_race_and_history_is_bounded() -> Result<()> {
        let registry = registry();
        let lease = registry.session(true)?;
        let offer = incoming_offer(1);
        registry.incoming(&lease.owner, &offer)?;
        registry.cancel(&offer.transfer_id)?;
        registry.published(&lease.owner, &offer.transfer_id, 4);
        registry.update(
            &lease.owner,
            &offer.transfer_id,
            1,
            "cancelled",
            "late cancel",
        );
        assert!(registry.snapshot().contains("\t4\t4\tcompleted\t"));
        let late = incoming_offer(80);
        registry.incoming(&lease.owner, &late)?;
        registry.update(
            &lease.owner,
            &late.transfer_id,
            1,
            "failed",
            "Session disconnected",
        );
        registry.published(&lease.owner, &late.transfer_id, 4);
        assert!(!registry.snapshot().contains("Session disconnected"));
        for id in 2..70 {
            let offer = incoming_offer(id);
            registry.incoming(&lease.owner, &offer)?;
            registry.update(&lease.owner, &offer.transfer_id, 4, "completed", "");
        }
        assert_eq!(registry.state.lock().unwrap().rows.len(), HISTORY);
        Ok(())
    }

    #[test]
    fn old_session_requests_and_drops_cannot_affect_the_replacement() -> Result<()> {
        let registry = registry();
        let sessions = registry.session_changes();
        let old = registry.session(true)?;
        let (id, owner) = registry.enqueue(&old.owner, Path::new("old.bin"), 4)?;
        let new = registry.session(true)?;
        assert!(registry.enqueue(&owner, Path::new("stale.bin"), 4).is_err());
        let (new_id, new_owner) = registry.enqueue(&new.owner, Path::new("new.bin"), 4)?;
        assert!(!registry.claim(&owner, &id, 4));
        assert!(registry.cancel(&id).is_err());
        drop(old);
        assert!(registry.claim(&new_owner, &new_id, 4));
        assert!(registry
            .state
            .lock()
            .unwrap()
            .live
            .as_ref()
            .is_some_and(|(live, _)| live == &new.owner));
        assert_eq!(sessions.borrow().as_deref(), Some(new.owner.as_str()));
        drop(new);
        assert!(sessions.borrow().is_none());
        Ok(())
    }

    #[test]
    fn duplicate_offer_cannot_replace_an_active_row_or_evict_active_state() -> Result<()> {
        let registry = registry();
        let lease = registry.session(true)?;
        for id in 1..=HISTORY as u8 {
            registry.incoming(&lease.owner, &incoming_offer(id))?;
        }
        assert!(registry.incoming(&lease.owner, &incoming_offer(1)).is_err());
        assert!(registry
            .incoming(&lease.owner, &incoming_offer(99))
            .is_err());
        assert_eq!(registry.state.lock().unwrap().rows.len(), HISTORY);
        Ok(())
    }

    #[tokio::test]
    async fn scheduler_waits_for_the_matching_result_and_has_a_bounded_deadline() -> Result<()> {
        let path =
            std::env::temp_dir().join(format!("conduit-scheduler-{}.bin", std::process::id()));
        std::fs::write(&path, b"payload")?;
        let (completion, mut result) = tokio::sync::oneshot::channel();
        let (progress, _updates) = watch::channel(control::TransferProgress {
            transferred: 0,
            total: 7,
        });
        let request = control::SendRequest {
            id: vec![9; 16],
            owner: "session".into(),
            path: path.clone(),
            completion: Some(completion),
            progress,
        };
        let mut sender = Sending::start(request);
        let offer = sender.frames.recv().await.unwrap()?;
        assert_eq!(offer.kind, pb::Kind::FileOffer);
        let mut ack = pb::FileResult {
            transfer_id: vec![9; 16],
            success: true,
            error: String::new(),
            name: String::new(),
            cancelled: false,
        };
        assert!(
            !sender.accepts(&ack),
            "early success cannot complete an unsent file"
        );
        let chunk = sender.frames.recv().await.unwrap()?;
        assert_eq!(chunk.kind, pb::Kind::FileChunk);
        sender.sent = chunk.bytes;
        sender.total = chunk.total;
        let now = tokio::time::Instant::now();
        sender.waiting = Some(now + RESULT_TIMEOUT);
        assert!(sender.accepts(&ack));
        ack.transfer_id = vec![8; 16];
        assert!(!sender.accepts(&ack));
        assert!(matches!(
            result.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        assert!(!sender.expired(now + RESULT_TIMEOUT - Duration::from_millis(1)));
        assert!(sender.expired(now + RESULT_TIMEOUT));
        sender.cancel_sent = true;
        ack.transfer_id = vec![9; 16];
        assert!(sender.accepts(&ack), "publication wins cancellation race");
        sender.request.finish(Ok(()));
        assert!(result.await?.is_ok());
        drop(sender);
        std::fs::remove_file(path)?;
        Ok(())
    }
}
