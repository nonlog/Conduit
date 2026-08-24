//! conduit daemon — Windows side.
//!
//! M0 scope: advertise over mDNS, accept one LAN peer, Noise XX, keep the socket
//! honest, sync text clipboard both ways. The shape here is chosen so image sync adds
//! neither a thread nor a timer per message.

mod advert;
mod clip;
mod wire;

use anyhow::{Context, Result};
use prost::Message as _;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tracing::{info, warn};
use wire::{pb, Session};

const PORT: u16 = 41112;
/// Silence this long and we make the peer prove the path is alive. KDE Connect's
/// bug 476747 is the counter-example: OS defaults meant ~7875 s to notice a dead peer.
const IDLE_PING: Duration = Duration::from_secs(60);
const PONG_DEADLINE: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Metrics {
    created: AtomicU64,
    closed: AtomicU64,
    frames_in: AtomicU64,
    frames_out: AtomicU64,
}

/// Increments `closed` on drop — normal return, error, panic or task abort alike —
/// so `created == closed` after quiesce holds without bookkeeping at every exit.
/// That invariant is the entire reason this project exists.
struct SessionGuard(Arc<Metrics>);

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let closed = self.0.closed.fetch_add(1, Ordering::Relaxed) + 1;
        info!(
            created = self.0.created.load(Ordering::Relaxed),
            closed,
            frames_in = self.0.frames_in.load(Ordering::Relaxed),
            frames_out = self.0.frames_out.load(Ordering::Relaxed),
            "session closed"
        );
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "conduit_daemon=info".into()),
        )
        .init();

    let dir = config_dir()?;
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let identity = wire::load_or_create_identity(&dir.join("identity.bin"))?;
    let device_id = wire::device_id(&identity.public);
    let fingerprint = wire::fingerprint(&identity.public);
    info!(id = %device_id, %fingerprint, "identity");

    let metrics = Arc::new(Metrics::default());
    // One listener thread for the process, started before the socket so a clip copied
    // during the first handshake is already in the ring.
    let bridge = Arc::new(clip::Bridge::start()?);
    let listener = TcpListener::bind(("0.0.0.0", PORT)).await?;
    info!(port = PORT, "listening");
    // Bound, not dropped: this is what the phone's discovery burst finds. Advertised
    // only once the socket is accepting, so a resolve is never answered by a refusal.
    let _advert = advert::Advert::start(PORT, &device_id, &fingerprint)?;

    // M0 carries exactly one peer. A reconnect must win, so the previous session is
    // dropped rather than the new one refused: a half-open socket would otherwise
    // lock the phone out until its own keepalive gave up.
    let mut active: Option<tokio::task::JoinHandle<()>> = None;
    loop {
        let (stream, peer) = listener.accept().await?;
        if let Some(previous) = active.take() {
            previous.abort();
            let _ = previous.await; // guard drop runs here — closed is counted
        }
        metrics.created.fetch_add(1, Ordering::Relaxed);
        let metrics = metrics.clone();
        let local_priv = identity.private.clone();
        let bridge = bridge.clone();
        active = Some(tokio::spawn(async move {
            let _guard = SessionGuard(metrics.clone());
            if let Err(e) = serve(stream, peer, &local_priv, &metrics, &bridge).await {
                warn!(%peer, error = %e, "session ended");
            }
        }));
    }
}

async fn serve(
    mut stream: TcpStream,
    peer: SocketAddr,
    local_priv: &[u8],
    metrics: &Metrics,
    bridge: &clip::Bridge,
) -> Result<()> {
    stream.set_nodelay(true)?;
    set_keepalive(&stream)?;

    let mut session = Session::handshake(&mut stream, local_priv, false).await?;
    info!(%peer, id = %wire::device_id(&session.peer_static), "session up");
    // Subscribed after the handshake so the session does not replay clips copied
    // while it was still connecting.
    let mut clips = bridge.subscribe();

    loop {
        // `recv` is cancel-safe, which is what makes racing it legal. `send` is not,
        // but a `select!` branch body runs to completion once chosen, so the sends
        // below are never cancelled mid-frame.
        let envelope = tokio::select! {
            result = tokio::time::timeout(IDLE_PING, session.recv(&mut stream)) => match result {
                Ok(envelope) => envelope?,
                Err(_) => {
                    session.send(&mut stream, pb::Kind::Ping, &[]).await?;
                    metrics.frames_out.fetch_add(1, Ordering::Relaxed);
                    tokio::time::timeout(PONG_DEADLINE, session.recv(&mut stream))
                        .await
                        .context("peer silent past pong deadline")??
                }
            },
            local = clips.recv() => {
                match local {
                    Ok(text) => {
                        let clip = pb::ClipText {
                            timestamp_ms: now_ms(),
                            mime: "text/plain".into(),
                            text,
                        };
                        session
                            .send(&mut stream, pb::Kind::ClipText, &clip.encode_to_vec())
                            .await?;
                        metrics.frames_out.fetch_add(1, Ordering::Relaxed);
                    }
                    // The ring overwrote clips faster than this session drained them.
                    // Skipping is correct: only the newest clipboard state matters.
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "clip ring lagged")
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        anyhow::bail!("clipboard bridge is gone")
                    }
                }
                continue;
            }
        };
        metrics.frames_in.fetch_add(1, Ordering::Relaxed);

        match envelope.kind() {
            pb::Kind::Ping => {
                session.send(&mut stream, pb::Kind::Pong, &[]).await?;
                metrics.frames_out.fetch_add(1, Ordering::Relaxed);
            }
            pb::Kind::Pong => {}
            pb::Kind::ClipText => {
                let clip = pb::ClipText::decode(&envelope.payload[..])?;
                info!(bytes = clip.text.len(), mime = %clip.mime, "clip text in");
                // A clipboard failure is the peer's problem, not the session's.
                if let Err(e) = bridge.apply(&clip.text) {
                    warn!(error = %e, "could not set the clipboard");
                }
            }
            other => warn!(?other, "unhandled kind"),
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 30 s idle / 10 s probe / 3 retries. Windows' `SIO_KEEPALIVE_VALS` has no retry
/// count, so socket2 only exposes `with_retries` elsewhere.
fn set_keepalive(stream: &TcpStream) -> Result<()> {
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(30))
        .with_interval(Duration::from_secs(10));
    #[cfg(not(windows))]
    let keepalive = keepalive.with_retries(3);
    socket2::SockRef::from(stream).set_tcp_keepalive(&keepalive)?;
    Ok(())
}

fn config_dir() -> Result<PathBuf> {
    Ok(PathBuf::from(std::env::var("LOCALAPPDATA").context("LOCALAPPDATA is unset")?).join("Conduit"))
}
