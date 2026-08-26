//! conduit daemon — Windows side.
//!
//! M0 scope: advertise over mDNS, accept one LAN peer, Noise XX, keep the socket
//! honest, sync text clipboard both ways. The shape here is chosen so image sync adds
//! neither a thread nor a timer per message.

mod advert;
mod autostart;
mod clip;
mod config;
mod control;
mod explorer;
mod file;
mod image;
mod status;
mod toast;
mod wire;

use anyhow::{bail, Context, Result};
use prost::Message as _;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, Mutex};
use tracing::{info, warn};
use wire::{pb, Session};

const PORT: u16 = 41112;
/// Default relay while the multi-relay fleet is not deployed everywhere yet.
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
struct SessionGuard {
    metrics: Arc<Metrics>,
    status: Arc<status::StatusFile>,
}

struct RelayArrival {
    stream: TcpStream,
    endpoint: String,
}

struct PendingOutbound {
    transfer_id: Vec<u8>,
    name: String,
    completion: tokio::sync::oneshot::Sender<std::result::Result<(), String>>,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.status.disconnected();
        let closed = self.metrics.closed.fetch_add(1, Ordering::Relaxed) + 1;
        info!(
            created = self.metrics.created.load(Ordering::Relaxed),
            closed,
            frames_in = self.metrics.frames_in.load(Ordering::Relaxed),
            frames_out = self.metrics.frames_out.load(Ordering::Relaxed),
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

    let mut args = std::env::args_os();
    let _exe = args.next();
    if let Some(command) = args.next() {
        if command == "send" {
            let path = args.next().context("usage: conduit-daemon send <file>")?;
            if args.next().is_some() {
                bail!("usage: conduit-daemon send <file>");
            }
            let path = control::queue(&PathBuf::from(path)).await?;
            println!("Sent to phone: {}", path.display());
            return Ok(());
        }
        if command == "autostart" {
            let action = args.next().context("usage: conduit-daemon autostart <install|remove|status>")?;
            if args.next().is_some() {
                bail!("usage: conduit-daemon autostart <install|remove|status>");
            }
            match action.to_string_lossy().as_ref() {
                "install" => println!("Autostart installed: {}", autostart::install()?),
                "remove" => println!(
                    "Autostart {}",
                    if autostart::remove()? { "removed" } else { "was not installed" }
                ),
                "status" => match autostart::status() {
                    Some(value) => println!("Autostart installed: {value}"),
                    None => println!("Autostart not installed"),
                },
                _ => bail!("usage: conduit-daemon autostart <install|remove|status>"),
            }
            return Ok(());
        }
        if command == "explorer" {
            let action = args.next().context("usage: conduit-daemon explorer <install|remove|status>")?;
            if args.next().is_some() {
                bail!("usage: conduit-daemon explorer <install|remove|status>");
            }
            match action.to_string_lossy().as_ref() {
                "install" => println!("Explorer integration installed: {}", explorer::install()?),
                "remove" => println!(
                    "Explorer integration {}",
                    if explorer::remove()? { "removed" } else { "was not installed" }
                ),
                "status" => match explorer::status() {
                    Some(value) => println!("Explorer integration installed: {value}"),
                    None => println!("Explorer integration not installed"),
                },
                _ => bail!("usage: conduit-daemon explorer <install|remove|status>"),
            }
            return Ok(());
        }
        if command == "config" {
            let dir = config_dir()?;
            std::fs::create_dir_all(&dir)?;
            let mut config = config::Config::load(&dir)?;
            let action = args.next().context(
                "usage: conduit-daemon config <show|relay-proxy <value|off>|relays <list|off>>",
            )?;
            match action.to_string_lossy().as_ref() {
                "show" if args.next().is_none() => {
                    println!("Config file: {}", dir.join("config.txt").display());
                    println!(
                        "relay_proxy={}",
                        config.relay_proxy.as_deref().filter(|v| !v.is_empty()).unwrap_or("off")
                    );
                    println!(
                        "relays={}",
                        config.relays.as_deref().unwrap_or(RELAY)
                    );
                    if std::env::var_os("CONDUIT_RELAY_PROXY").is_some()
                        || std::env::var_os("CONDUIT_RELAYS").is_some()
                        || std::env::var_os("CONDUIT_RELAY").is_some()
                    {
                        println!("Note: one or more CONDUIT_* environment variables override this file.");
                    }
                }
                "relay-proxy" => {
                    let value = args.next().context("usage: conduit-daemon config relay-proxy <value|off>")?;
                    if args.next().is_some() {
                        bail!("usage: conduit-daemon config relay-proxy <value|off>");
                    }
                    let value = value.to_string_lossy();
                    config.relay_proxy = Some(if value.eq_ignore_ascii_case("off") {
                        String::new()
                    } else {
                        value.into_owned()
                    });
                    println!("Saved {}", config.save(&dir)?.display());
                    println!("Restart the daemon to apply this change.");
                }
                "relays" => {
                    let value = args.next().context("usage: conduit-daemon config relays <list|off>")?;
                    if args.next().is_some() {
                        bail!("usage: conduit-daemon config relays <list|off>");
                    }
                    let value = value.to_string_lossy();
                    config.relays = Some(if value.eq_ignore_ascii_case("off") {
                        String::new()
                    } else {
                        value.into_owned()
                    });
                    println!("Saved {}", config.save(&dir)?.display());
                    println!("Restart the daemon to apply this change.");
                }
                _ => bail!(
                    "usage: conduit-daemon config <show|relay-proxy <value|off>|relays <list|off>>"
                ),
            }
            return Ok(());
        }
        if command == "status" && args.next().is_none() {
            let dir = config_dir()?;
            // The daemon itself owns 0.0.0.0:41112. Binding that exact address/port is a
            // side-effect-free, on-demand liveness check; binding only loopback can succeed on
            // Windows even while the wildcard listener exists and would therefore misreport.
            let daemon_running = std::net::TcpListener::bind(("0.0.0.0", PORT)).is_err();
            println!("daemon={}", if daemon_running { "running" } else { "stopped" });
            if let Some(snapshot) = status::read(&dir) {
                print!("{snapshot}");
            }
            return Ok(());
        }
        bail!(
            "unknown command {:?}; usage: conduit-daemon [send <file> | status | autostart <install|remove|status> | explorer <install|remove|status> | config ...]",
            command
        );
    }

    let dir = config_dir()?;
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let user_config = config::Config::load(&dir)?;
    let desktop_status = Arc::new(status::StatusFile::new(&dir)?);
    let identity = wire::load_or_create_identity(&dir.join("identity.bin"))?;
    let device_id = wire::device_id(&identity.public);
    let fingerprint = wire::fingerprint(&identity.public);
    info!(id = %device_id, %fingerprint, "identity");

    // Bind before creating clipboard/toast/control workers. Besides being the LAN listener, this
    // is the process's zero-extra-resource single-instance gate: a manual launch racing the Run
    // entry exits here before it can own any long-lived Conduit resource.
    let listener = TcpListener::bind(("0.0.0.0", PORT))
        .await
        .with_context(|| format!("binding Conduit listener on port {PORT}; is another daemon running?"))?;
    info!(port = PORT, "listening");

    let metrics = Arc::new(Metrics::default());
    // One listener thread for the process, started before the socket so a clip copied
    // during the first handshake is already in the ring.
    let bridge = Arc::new(clip::Bridge::start()?);
    // Likewise one toast thread. A failure here is not fatal: clipboard sync is still
    // worth having on a machine where the notification platform will not cooperate.
    let toasts = match toast::Notifier::start(&dir) {
        Ok(notifier) => Some(Arc::new(notifier)),
        Err(e) => {
            warn!(error = %e, "toasts unavailable, mirroring notifications is disabled");
            None
        }
    };
    // One local command queue for the process. File bytes never pass through this IPC; callers
    // name a local path and the resident daemon opens/streams it through its one live session.
    let (outbound_tx, outbound_rx) = mpsc::channel::<control::SendRequest>(16);
    let outbound = Arc::new(Mutex::new(outbound_rx));
    tokio::spawn(async move {
        if let Err(e) = control::serve(outbound_tx).await {
            warn!(error = %e, "local control pipe stopped");
        }
    });
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
    let relays = user_config.resolved_relays(RELAY);
    let relay_proxy = user_config.resolved_proxy();
    info!(
        relays = ?relays,
        proxy = relay_proxy.as_deref().unwrap_or("direct"),
        "relay configuration"
    );
    let (relay_tx, mut relay_rx) =
        tokio::sync::mpsc::channel::<RelayArrival>((relays.len().max(1) * 2).max(2));
    for endpoint in &relays {
        let rendezvous = device_id.clone();
        info!(%endpoint, "parking at relay");
        tokio::spawn(park_forever(
            endpoint.clone(),
            rendezvous,
            relay_tx.clone(),
            relay_proxy.clone(),
        ));
    }
    // Only the per-relay workers own senders. With no configured relays this closes the
    // receiver and permanently disables its select branch without a polling special case.
    drop(relay_tx);

    let mut active: Option<tokio::task::JoinHandle<()>> = None;
    loop {
        let (stream, peer, via, relay_endpoint) = tokio::select! {
            accepted = listener.accept() => {
                let (stream, peer) = accepted?;
                (stream, peer, "lan", None)
            }
            Some(arrival) = relay_rx.recv() => {
                let peer = arrival.stream.peer_addr()?;
                (arrival.stream, peer, "relay", Some(arrival.endpoint))
            }
        };
        info!(%peer, via, relay = relay_endpoint.as_deref().unwrap_or("-"), "peer arriving");
        if let Some(previous) = active.take() {
            previous.abort();
            let _ = previous.await; // guard drop runs here — closed is counted
        }
        let created = metrics.created.fetch_add(1, Ordering::Relaxed) + 1;
        info!(
            created,
            closed = metrics.closed.load(Ordering::Relaxed),
            "session created"
        );
        let metrics = metrics.clone();
        let local_priv = identity.private.clone();
        let bridge = bridge.clone();
        let toasts = toasts.clone();
        let outbound = outbound.clone();
        let relay_endpoint = relay_endpoint.clone();
        let desktop_status = desktop_status.clone();
        active = Some(tokio::spawn(async move {
            let _guard = SessionGuard {
                metrics: metrics.clone(),
                status: desktop_status.clone(),
            };
            if let Err(e) = serve(
                stream,
                peer,
                &local_priv,
                &metrics,
                &bridge,
                &outbound,
                toasts.as_deref(),
                via,
                relay_endpoint.as_deref(),
                &desktop_status,
            )
            .await
            {
                warn!(%peer, error = %e, "session ended");
            }
        }));
    }
}

/// Keeps exactly one connection parked at the relay for the life of the process.
///
/// Re-parks immediately after handing a stream over, so a reconnecting phone always
/// finds a partner already waiting instead of racing the desktop to the rendezvous.
/// The relay pairs whoever presents the id, so a stale park is harmless: the next
/// arrival is spliced to the newest one.
async fn park_forever(
    endpoint: String,
    rendezvous: String,
    tx: tokio::sync::mpsc::Sender<RelayArrival>,
    relay_proxy: Option<String>,
) {
    loop {
        match wire::park(&endpoint, &rendezvous, relay_proxy.as_deref()).await {
            Ok(stream) => {
                if let Err(e) = set_keepalive(&stream) {
                    warn!(error = %e, "relay stream without keepalive");
                }
                // Closed channel means main returned; nothing left to serve.
                if tx
                    .send(RelayArrival {
                        stream,
                        endpoint: endpoint.clone(),
                    })
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Err(e) => {
                warn!(%endpoint, error = %e, "relay unreachable");
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
    outbound: &Arc<Mutex<mpsc::Receiver<control::SendRequest>>>,
    toasts: Option<&toast::Notifier>,
    // "lan" or "relay". Only the keepalive interval and the log line care.
    path: &'static str,
    relay_endpoint: Option<&str>,
    desktop_status: &status::StatusFile,
) -> Result<()> {
    stream.set_nodelay(true)?;
    set_keepalive(&stream)?;
    let idle_ping = if path == "relay" { RELAY_IDLE_PING } else { IDLE_PING };

    let mut session = Session::handshake(&mut stream, local_priv, false).await?;
    info!(
        %peer,
        id = %wire::device_id(&session.peer_static),
        via = path,
        relay = relay_endpoint.unwrap_or("-"),
        "session up"
    );
    desktop_status.linked(
        &wire::device_id(&session.peer_static),
        path,
        relay_endpoint,
    );
    // The phone has no other way to learn this machine's name. mDNS carries it, but a
    // relay session never sees an mDNS record — and off-LAN is precisely when a phone
    // showing "the desktop" instead of a name is least useful. Sent unprompted and
    // unanswered: the handshake already settled who the peer is.
    let hello = pb::PairRequest {
        device_id: wire::device_id(&session.peer_static),
        device_name: advert::hostname(),
        static_pub: Vec::new(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    session
        .send(&mut stream, pb::Kind::PairRequest, &hello.encode_to_vec())
        .await?;
    metrics.frames_out.fetch_add(1, Ordering::Relaxed);
    // Subscribed after the handshake so the session does not replay clips copied
    // while it was still connecting.
    let mut clips = bridge.subscribe();
    // At most one image in flight, and it dies with the session: a peer that vanishes
    // mid-transfer cannot leave a partial buffer behind for the next one to inherit.
    let mut incoming: Option<image::Assembly> = None;
    // Same rule for a file, with teeth: `Incoming`'s Drop deletes the partial, so a
    // session ending mid-transfer cannot leave scratch files in Downloads.
    let mut arriving: Option<file::Incoming> = None;
    // One desktop->phone file may be awaiting its receiver-side publication result. Other local
    // requests stay in the bounded control queue until this completes, so results cannot cross.
    let mut pending_outbound: Option<PendingOutbound> = None;

    loop {
        // Time left before this side owes the peer a frame. Measured from our last
        // *send*, not from the last thing we heard: the phone's read deadline can only
        // be satisfied by us speaking, so a phone that chats steadily while hearing
        // nothing back used to tear down a perfectly healthy session on the dot. It
        // also still catches a dead peer, because a ping demands a pong.
        let quiet = idle_ping.saturating_sub(session.quiet_for());
        // `recv` is cancel-safe, which is what makes racing it legal. `send` is not,
        // but a `select!` branch body runs to completion once chosen, so the sends
        // below are never cancelled mid-frame.
        let envelope = tokio::select! {
            result = tokio::time::timeout(quiet, session.recv(&mut stream)) => match result {
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
                        let frames = image::send(&mut session, &mut stream, &png, false, false).await?;
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
            local_file = async {
                let mut rx = outbound.lock().await;
                rx.recv().await
            }, if pending_outbound.is_none() => {
                let Some(request) = local_file else {
                    bail!("local outbound file queue closed")
                };
                let path = request.path;
                // Refuse a bad/missing local path before FILE_OFFER reaches the phone. Once an
                // offer is on the wire, a read/send error ends this session so Android drops its
                // pending MediaStore row rather than inheriting a partial into the next session.
                match file::Outbound::open(&path).await {
                    Ok(outbound_file) => {
                        let name = outbound_file.name().to_string();
                        let transfer_id = outbound_file.transfer_id().to_vec();
                        match outbound_file.send(&mut session, &mut stream).await {
                            Ok(frames) => {
                                metrics.frames_out.fetch_add(frames, Ordering::Relaxed);
                                pending_outbound = Some(PendingOutbound {
                                    transfer_id,
                                    name,
                                    completion: request.completion,
                                });
                            }
                            Err(e) => {
                                let message = format!("sending {name} to the phone: {e:#}");
                                let _ = request.completion.send(Err(message.clone()));
                                return Err(e).with_context(|| format!("sending {name} to the phone"));
                            }
                        }
                    }
                    Err(e) => {
                        let message = format!("opening {}: {e:#}", path.display());
                        let _ = request.completion.send(Err(message));
                        warn!(path = %path.display(), error = %e, "local file was not sent");
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
                            screenshot = header.screenshot,
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
                        let screenshot = assembly.is_screenshot();
                        incoming = None;
                        info!(bytes = png.len(), photo, screenshot, "image complete");
                        if photo || screenshot {
                            // Never the clipboard. Camera photos and screenshots are capture
                            // events, not copy events. A screenshot also sets photo=true for
                            // backward safety, while the explicit bit selects the right toast.
                            match toasts {
                                Some(toasts) => {
                                    match tokio::task::spawn_blocking(move || stage_capture(&png))
                                        .await
                                    {
                                        Ok(Ok(path)) => {
                                            if screenshot {
                                                toasts.post(toast::Cmd::Screenshot { path });
                                            } else {
                                                toasts.post(toast::Cmd::Photo { path });
                                            }
                                        }
                                        Ok(Err(e)) => warn!(error = %e, "could not stage the capture"),
                                        Err(e) => warn!(error = %e, "capture staging task failed"),
                                    }
                                }
                                None => info!("capture dropped, toasts are unavailable"),
                            }
                        } else {
                            // Decode, encode and two clipboard opens: too slow for a
                            // worker, and COM work regardless.
                            let bridge = bridge.clone();
                            match tokio::task::spawn_blocking(move || {
                                // Normalise first: the phone sends a camera JPEG as-is,
                                // and the PNG clipboard format must hold a real PNG.
                                let png = image::to_png(&png)?;
                                bridge.apply_image(&png)
                            })
                            .await
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
                        package: notif.package.clone(),
                        app: if notif.app_name.is_empty() { notif.package } else { notif.app_name },
                        title: notif.title,
                        body: notif.text,
                        app_icon: notif.app_icon_png,
                        avatar: notif.large_icon_png,
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
            // The phone announcing itself. Nothing to do with it on this side yet — the
            // desktop already knows which peer it is talking to — but decoding it keeps it
            // out of the "unhandled kind" log.
            pb::Kind::PairRequest => {
                let hello = pb::PairRequest::decode(&envelope.payload[..])?;
                info!(name = %hello.device_name, version = %hello.version, "peer named itself");
                desktop_status.peer_name(&hello.device_name);
            }
            // A file, same leniency as an image: a refused offer costs the transfer and
            // nothing else. The session is also carrying the clipboard, and a phone that
            // shares a 600 MB video must not knock it out.
            pb::Kind::FileOffer => {
                let offer = pb::FileOffer::decode(&envelope.payload[..])?;
                let dir = match file::downloads() {
                    Ok(dir) => dir,
                    Err(e) => {
                        warn!(error = %e, "no Downloads folder, file refused");
                        arriving = None;
                        continue;
                    }
                };
                match file::Incoming::begin(&offer, &dir) {
                    Ok(rx) => {
                        info!(
                            name = rx.name(),
                            bytes = offer.total_bytes,
                            chunks = offer.chunk_count,
                            mime = %offer.mime,
                            "file in, receiving"
                        );
                        arriving = Some(rx);
                    }
                    Err(e) => {
                        warn!(error = %e, "refused a file offer");
                        arriving = None;
                    }
                }
            }
            pb::Kind::FileChunk => {
                let chunk = pb::FileChunk::decode(&envelope.payload[..])?;
                let Some(rx) = arriving.as_mut() else {
                    warn!(index = chunk.index, "file chunk with no offer, dropped");
                    continue;
                };
                match rx.push(&chunk) {
                    Ok(None) => {}
                    Ok(Some(path)) => {
                        arriving = None;
                        match toasts {
                            Some(toasts) => toasts.post(toast::Cmd::File { path }),
                            None => info!("file arrived, but toasts are unavailable to say so"),
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "file transfer dropped");
                        // Assigning None runs the Drop that deletes the partial.
                        arriving = None;
                    }
                }
            }
            pb::Kind::FileResult => {
                let result = pb::FileResult::decode(&envelope.payload[..])?;
                let Some(pending) = pending_outbound.take() else {
                    warn!("file result arrived with no desktop transfer pending");
                    continue;
                };
                if result.transfer_id != pending.transfer_id {
                    warn!(
                        name = %pending.name,
                        "file result belongs to a different transfer, ignored"
                    );
                    pending_outbound = Some(pending);
                    continue;
                }
                if result.success {
                    info!(name = %pending.name, "phone published file");
                    let _ = pending.completion.send(Ok(()));
                } else {
                    let reason = if result.error.is_empty() {
                        "receiver refused the file".to_string()
                    } else {
                        result.error
                    };
                    warn!(name = %pending.name, error = %reason, "phone refused file");
                    let _ = pending.completion.send(Err(reason));
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

/// Writes the newest phone capture where the shell and Snipping Tool can both read it.
///
/// One file, overwritten each time, which is the same bound as the single capture toast
/// that names it. No transcode: the toast image loader and Snipping Tool each read JPEG
/// happily, the phone already downscaled it, and re-encoding a photograph as PNG would
/// multiply its size for nothing — local toast images are capped at 3 MB.
fn stage_capture(bytes: &[u8]) -> Result<PathBuf> {
    let (name, stale) = if bytes.starts_with(b"\x89PNG") {
        ("capture.png", "capture.jpg")
    } else {
        ("capture.jpg", "capture.png")
    };
    let dir = config_dir()?;
    // Switching between a JPEG camera photo and a PNG screenshot must not leave a second
    // staged capture behind. Best effort: the old toast/token is replaced in the same slot.
    let _ = std::fs::remove_file(dir.join(stale));
    let path = dir.join(name);
    std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
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
