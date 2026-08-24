//! conduit daemon — Windows side.
//!
//! M0 scope: advertise over mDNS, accept one LAN peer, Noise XX, keep the socket
//! honest, sync text clipboard both ways. The shape here is chosen so image sync adds
//! neither a thread nor a timer per message.

mod advert;
mod clip;
mod image;
mod toast;
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
/// The relay this desktop parks at when the phone is not on the LAN. Ours, in Tokyo:
/// lowest latency of the four hosts and the only one reachable without going through a
/// local proxy. `CONDUIT_RELAY=` (empty) turns the relay path off entirely.
const RELAY: &str = "tyo.414222.xyz:41113";
/// ponytail: flat retry, no escalation. The desktop is on mains power and the relay is
/// ours, so there is nothing to be polite to; add backoff if it ever rate-limits.
const RELAY_RETRY: Duration = Duration::from_secs(15);
/// Silence this long and we make the peer prove the path is alive. KDE Connect's
/// bug 476747 is the counter-example: OS defaults meant ~7875 s to notice a dead peer.
const IDLE_PING: Duration = Duration::from_secs(60);
/// Over the relay the peer is on cellular or a foreign network, where every ping is a
/// radio wake on a battery. Four an hour instead of sixty, at the cost of noticing a
/// silently dead tunnel later. The phone's read deadline is 2.5x this and follows it.
const RELAY_IDLE_PING: Duration = Duration::from_secs(240);
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
    // Likewise one toast thread. A failure here is not fatal: clipboard sync is still
    // worth having on a machine where the notification platform will not cooperate.
    let toasts = match toast::Notifier::start() {
        Ok(notifier) => Some(Arc::new(notifier)),
        Err(e) => {
            warn!(error = %e, "toasts unavailable, mirroring notifications is disabled");
            None
        }
    };
    let listener = TcpListener::bind(("0.0.0.0", PORT)).await?;
    info!(port = PORT, "listening");
    // Bound, not dropped: this is what the phone's discovery burst finds. Advertised
    // only once the socket is accepting, so a resolve is never answered by a refusal.
    let _advert = advert::Advert::start(PORT, &device_id, &fingerprint)?;

    // M0 carries exactly one peer. A reconnect must win, so the previous session is
    // dropped rather than the new one refused: a half-open socket would otherwise
    // lock the phone out until its own keepalive gave up.
    //
    // Two ways in now, and the difference ends here: a relay stream is spliced to the
    // phone by a process that cannot read it, so from `serve`'s point of view it is an
    // ordinary socket carrying an ordinary Noise session.
    let (relay_tx, mut relay_rx) = tokio::sync::mpsc::channel::<TcpStream>(1);
    match relay_endpoint() {
        Some(endpoint) => {
            let rendezvous = device_id.clone();
            info!(%endpoint, "parking at the relay");
            tokio::spawn(park_forever(endpoint, rendezvous, relay_tx));
        }
        // Dropping the sender closes the channel, which permanently disables the
        // `select!` branch below rather than leaving it to fire on every poll.
        None => drop(relay_tx),
    }

    let mut active: Option<tokio::task::JoinHandle<()>> = None;
    loop {
        let (stream, peer, via) = tokio::select! {
            accepted = listener.accept() => {
                let (stream, peer) = accepted?;
                (stream, peer, "lan")
            }
            Some(stream) = relay_rx.recv() => {
                let peer = stream.peer_addr()?;
                (stream, peer, "relay")
            }
        };
        info!(%peer, via, "peer arriving");
        if let Some(previous) = active.take() {
            previous.abort();
            let _ = previous.await; // guard drop runs here — closed is counted
        }
        metrics.created.fetch_add(1, Ordering::Relaxed);
        let metrics = metrics.clone();
        let local_priv = identity.private.clone();
        let bridge = bridge.clone();
        let toasts = toasts.clone();
        active = Some(tokio::spawn(async move {
            let _guard = SessionGuard(metrics.clone());
            if let Err(e) = serve(stream, peer, &local_priv, &metrics, &bridge, toasts.as_deref(), via).await {
                warn!(%peer, error = %e, "session ended");
            }
        }));
    }
}

/// `CONDUIT_RELAY` overrides the built-in endpoint; setting it empty disables the relay.
fn relay_endpoint() -> Option<String> {
    match std::env::var("CONDUIT_RELAY") {
        Ok(value) if value.trim().is_empty() => None,
        Ok(value) => Some(value),
        Err(_) => Some(RELAY.to_string()),
    }
}

/// Keeps exactly one connection parked at the relay for the life of the process.
///
/// Re-parks immediately after handing a stream over, so a reconnecting phone always
/// finds a partner already waiting instead of racing the desktop to the rendezvous.
/// The relay pairs whoever presents the id, so a stale park is harmless: the next
/// arrival is spliced to the newest one.
async fn park_forever(endpoint: String, rendezvous: String, tx: tokio::sync::mpsc::Sender<TcpStream>) {
    loop {
        match wire::park(&endpoint, &rendezvous).await {
            Ok(stream) => {
                if let Err(e) = set_keepalive(&stream) {
                    warn!(error = %e, "relay stream without keepalive");
                }
                // Closed channel means main returned; nothing left to serve.
                if tx.send(stream).await.is_err() {
                    return;
                }
            }
            Err(e) => {
                warn!(error = %e, "relay unreachable");
                tokio::time::sleep(RELAY_RETRY).await;
            }
        }
    }
}

async fn serve(
    mut stream: TcpStream,
    peer: SocketAddr,
    local_priv: &[u8],
    metrics: &Metrics,
    bridge: &Arc<clip::Bridge>,
    toasts: Option<&toast::Notifier>,
    // "lan" or "relay". Only the keepalive interval and the log line care.
    path: &'static str,
) -> Result<()> {
    stream.set_nodelay(true)?;
    set_keepalive(&stream)?;
    let idle_ping = if path == "relay" { RELAY_IDLE_PING } else { IDLE_PING };

    let mut session = Session::handshake(&mut stream, local_priv, false).await?;
    info!(%peer, id = %wire::device_id(&session.peer_static), via = path, "session up");
    // Subscribed after the handshake so the session does not replay clips copied
    // while it was still connecting.
    let mut clips = bridge.subscribe();
    // At most one image in flight, and it dies with the session: a peer that vanishes
    // mid-transfer cannot leave a partial buffer behind for the next one to inherit.
    let mut incoming: Option<image::Assembly> = None;

    loop {
        // `recv` is cancel-safe, which is what makes racing it legal. `send` is not,
        // but a `select!` branch body runs to completion once chosen, so the sends
        // below are never cancelled mid-frame.
        let envelope = tokio::select! {
            result = tokio::time::timeout(idle_ping, session.recv(&mut stream)) => match result {
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
                    Ok(clip::Clip::Text(text)) => {
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
                    Ok(clip::Clip::Image(png)) => {
                        info!(bytes = png.len(), "clip image out");
                        let frames = image::send(&mut session, &mut stream, &png, false).await?;
                        metrics.frames_out.fetch_add(frames, Ordering::Relaxed);
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
            // A bad image is dropped, never fatal — same rule as a bad notification.
            // The peer controls every field here, so a refused header must cost this
            // side an allocation of zero and the session nothing at all.
            pb::Kind::ClipImageHeader => {
                let header = pb::ClipImageHeader::decode(&envelope.payload[..])?;
                match image::Assembly::begin(&header) {
                    Ok(assembly) => {
                        info!(
                            bytes = header.total_bytes,
                            chunks = header.chunk_count,
                            mime = %header.mime,
                            photo = header.photo,
                            "image in, receiving"
                        );
                        incoming = Some(assembly);
                    }
                    Err(e) => {
                        warn!(error = %e, "refused an image header");
                        incoming = None;
                    }
                }
            }
            pb::Kind::ClipImageChunk => {
                let chunk = pb::ClipImageChunk::decode(&envelope.payload[..])?;
                let Some(assembly) = incoming.as_mut() else {
                    warn!(index = chunk.index, "image chunk with no header, dropped");
                    continue;
                };
                match assembly.push(&chunk) {
                    Ok(None) => {}
                    Ok(Some(png)) => {
                        let photo = assembly.is_photo();
                        incoming = None;
                        info!(bytes = png.len(), photo, "image complete");
                        if photo {
                            // ponytail: a camera photo must not hijack the clipboard, and
                            // the toast path cannot carry a hero image yet, so it is
                            // logged and dropped. Wire it to toast::Cmd when that lands.
                            info!("photo received; hero-image toasts are not wired up yet");
                        } else {
                            // Decode, encode and two clipboard opens: too slow for a
                            // worker, and COM work regardless.
                            let bridge = bridge.clone();
                            match tokio::task::spawn_blocking(move || bridge.apply_image(&png)).await
                            {
                                Ok(Err(e)) => warn!(error = %e, "could not set the clipboard image"),
                                Err(e) => warn!(error = %e, "clipboard image task failed"),
                                Ok(Ok(())) => {}
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "image transfer dropped");
                        incoming = None;
                    }
                }
            }
            // A malformed notification is dropped, never fatal: the phone's shade must
            // not be able to end a session that is also carrying the clipboard.
            pb::Kind::NotifNew => {
                let notif = pb::NotifNew::decode(&envelope.payload[..])?;
                info!(app = %notif.app_name, pkg = %notif.package, "notif in");
                if let Some(toasts) = toasts {
                    toasts.post(toast::Cmd::Show {
                        key: notif.key,
                        app: if notif.app_name.is_empty() { notif.package } else { notif.app_name },
                        title: notif.title,
                        body: notif.text,
                    });
                }
            }
            pb::Kind::NotifUpdate => {
                let notif = pb::NotifUpdate::decode(&envelope.payload[..])?;
                if let Some(toasts) = toasts {
                    toasts.post(toast::Cmd::Update {
                        key: notif.key,
                        title: notif.title,
                        body: notif.text,
                    });
                }
            }
            pb::Kind::NotifRemove => {
                let notif = pb::NotifRemove::decode(&envelope.payload[..])?;
                if let Some(toasts) = toasts {
                    toasts.post(toast::Cmd::Hide { key: notif.key });
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
