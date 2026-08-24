//! conduit relay — a blind rendezvous for two peers that cannot reach each other.
//!
//! Both ends dial *outbound* TCP here, which is what crosses carrier NAT with no
//! traversal logic whatsoever: no ICE, no STUN, no TURN. Phone Link's 13.5 MB ICE agent
//! is the transport leak this project exists to escape, and this file is the entire
//! replacement for it.
//!
//! The relay is untrusted by construction. It sees a 47-byte preamble — `CDT1` plus a
//! rendezvous id that is already public — and then opaque Noise ciphertext. The XX
//! handshake runs phone-to-desktop *through* here, so session keys never exist on this
//! machine and it cannot read a clipboard or a notification even if it wanted to. There
//! is no config file, no database, and no state that outlives the process: a restart
//! just makes both peers redial.
//!
//! Pairing rule: the first connection presenting an id waits, and the next one
//! presenting the same id is spliced to it. Roles are not encoded, because the phone is
//! always the Noise initiator and the desktop always the responder — the relay never
//! needs to know which is which.
//!
//! Staleness needs no detection. A waiter whose TCP died is spliced to the next arrival,
//! the copy ends at once, the live peer sees EOF and redials. One wasted round trip
//! instead of a liveness probe. Waiters that are merely idle — a desktop waiting hours
//! for a phone — are the correct behaviour, and the kernel's own keepalive both reaps
//! the genuinely dead ones and refreshes the NAT mapping that keeps the live ones
//! reachable. No timer runs in this process.

use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tracing::{info, warn};

const MAGIC: [u8; 4] = *b"CDT1";

/// `BASE64URL(SHA256(static_pub))` unpadded is always 43 characters, so the preamble is
/// a fixed size: no length field, no delimiter, no parser to get wrong.
const ID_LEN: usize = 43;
const PREAMBLE: usize = MAGIC.len() + ID_LEN;

/// A connection that has not sent its preamble is holding an fd for nothing, and the
/// kernel's keepalive cannot help — such a peer is alive, merely silent.
const PREAMBLE_DEADLINE: Duration = Duration::from_secs(10);

/// One fd per waiter, so this ceiling bounds a buggy or hostile client rather than
/// rationing a real resource. Far past any plausible number of paired devices.
const MAX_WAITING: usize = 256;

const DEFAULT_PORT: u16 = 41113;

type Waiting = Mutex<HashMap<String, TcpStream>>;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "conduit_relay=info".into()),
        )
        .init();

    let port: u16 = match std::env::args().nth(1) {
        Some(arg) => arg.parse().with_context(|| format!("bad port {arg:?}"))?,
        None => DEFAULT_PORT,
    };
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;
    info!(port, "relay listening");

    let waiting: Arc<Waiting> = Arc::new(Mutex::new(HashMap::new()));
    let pairs = Arc::new(AtomicU64::new(0));
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            // Out of fds, or a reset between the SYN and the accept. Neither is a
            // reason to stop serving everybody else.
            Err(e) => {
                warn!(error = %e, "accept failed");
                continue;
            }
        };
        let waiting = waiting.clone();
        let pairs = pairs.clone();
        tokio::spawn(async move {
            if let Err(e) = arrive(stream, peer, &waiting, &pairs).await {
                warn!(%peer, error = %e, "dropped");
            }
        });
    }
}

/// One arrival: read its preamble, then either wait or splice.
///
/// The lock is held across map operations only, never across I/O, so a slow peer cannot
/// stall the rendezvous for anyone else.
async fn arrive(
    mut stream: TcpStream,
    peer: SocketAddr,
    waiting: &Waiting,
    pairs: &AtomicU64,
) -> Result<()> {
    stream.set_nodelay(true)?;
    keepalive(&stream)?;

    let mut buf = [0u8; PREAMBLE];
    tokio::time::timeout(PREAMBLE_DEADLINE, stream.read_exact(&mut buf))
        .await
        .context("no preamble before the deadline")??;
    if buf[..MAGIC.len()] != MAGIC {
        bail!("bad magic, not a conduit peer");
    }
    let id = std::str::from_utf8(&buf[MAGIC.len()..])
        .context("rendezvous id is not utf-8")?
        .to_owned();
    // Nothing outside the base64url alphabet can be a device id, and refusing it here
    // keeps arbitrary remote bytes out of the log.
    if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        bail!("rendezvous id is not base64url");
    }

    let mut waiter = {
        let mut map = waiting.lock().await;
        match map.remove(&id) {
            Some(waiter) => waiter,
            None => {
                if map.len() >= MAX_WAITING {
                    bail!("{} peers already waiting, refusing this one", map.len());
                }
                map.insert(id.clone(), stream);
                info!(%peer, id = %short(&id), waiting = map.len(), "waiting for a partner");
                return Ok(());
            }
        }
    };

    let n = pairs.fetch_add(1, Ordering::Relaxed) + 1;
    info!(%peer, id = %short(&id), pair = n, "spliced");
    // The only thing this process ever does with a payload. No parse, no inspection, no
    // buffer of its own beyond the two `copy_bidirectional` allocates.
    match tokio::io::copy_bidirectional(&mut waiter, &mut stream).await {
        Ok((up, down)) => info!(id = %short(&id), up, down, "pair closed"),
        // Expected, not exceptional: one side going away is how every session ends.
        Err(e) => info!(id = %short(&id), error = %e, "pair closed"),
    }
    Ok(())
}

/// A prefix, not the whole id. It is not secret — it is a hash of a public key — but a
/// log full of 43-character hashes is unreadable and there is no reason to make
/// correlating one device across days effortless.
fn short(id: &str) -> &str {
    &id[..id.len().min(12)]
}

/// 60 s idle / 10 s probe / 3 retries. The only liveness machinery in the relay, and it
/// belongs to the kernel: dead waiters are reaped and live ones have their NAT mapping
/// refreshed without this process owning a timer.
fn keepalive(stream: &TcpStream) -> Result<()> {
    let ka = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(60))
        .with_interval(Duration::from_secs(10));
    // Windows' SIO_KEEPALIVE_VALS has no retry count; this crate only ships to Linux,
    // but it is built and tested on the Windows dev box.
    #[cfg(not(windows))]
    let ka = ka.with_retries(3);
    socket2::SockRef::from(stream).set_tcp_keepalive(&ka)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    /// 43 characters, like every real device id.
    const ID: &str = "0123456789012345678901234567890123456789abc";

    /// A relay on an ephemeral port. The map comes back too, because a test that dials
    /// twice has to know the first arrival landed.
    async fn relay() -> (SocketAddr, Arc<Waiting>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let waiting: Arc<Waiting> = Arc::new(Mutex::new(HashMap::new()));
        let pairs = Arc::new(AtomicU64::new(0));
        let served = waiting.clone();
        tokio::spawn(async move {
            while let Ok((stream, peer)) = listener.accept().await {
                let (waiting, pairs) = (served.clone(), pairs.clone());
                tokio::spawn(async move {
                    let _ = arrive(stream, peer, &waiting, &pairs).await;
                });
            }
        });
        (addr, waiting)
    }

    async fn dial(addr: SocketAddr, id: &str) -> TcpStream {
        // A wrong-length id would stall on the fixed-size preamble and surface as
        // "never registered the waiter" ten seconds later, which reads like a relay bug.
        assert_eq!(id.len(), ID_LEN, "test id must be a realistic length");
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(&MAGIC).await.unwrap();
        s.write_all(id.as_bytes()).await.unwrap();
        s
    }

    /// Accepting is asynchronous, so a test dialling twice in a microsecond could leave
    /// both peers as waiters that never meet. Real peers are minutes apart; this is the
    /// only place the ordering has to be forced.
    async fn await_waiter(waiting: &Waiting, id: &str) {
        for _ in 0..1000 {
            if waiting.lock().await.contains_key(id) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        panic!("the relay never registered the waiter");
    }

    #[tokio::test]
    async fn two_peers_meet_and_bytes_cross_untouched() {
        let (addr, waiting) = relay().await;
        let mut a = dial(addr, ID).await;
        // The waiter writes before its partner exists. The kernel holds those bytes, so
        // the Noise initiator never has to wait for the relay to be "ready".
        a.write_all(b"noise-msg1").await.unwrap();
        await_waiter(&waiting, ID).await;
        let mut b = dial(addr, ID).await;

        let mut got = [0u8; 10];
        b.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"noise-msg1", "the relay altered the payload");

        b.write_all(b"noise-msg2!").await.unwrap();
        let mut back = [0u8; 11];
        a.read_exact(&mut back).await.unwrap();
        assert_eq!(&back, b"noise-msg2!", "the return direction is not spliced");
        assert!(waiting.lock().await.is_empty(), "a spliced pair must leave the map");
    }

    #[tokio::test]
    async fn a_stranger_is_refused_and_occupies_nothing() {
        let (addr, waiting) = relay().await;
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(b"HTTP").await.unwrap();
        s.write_all(&[b'x'; ID_LEN]).await.unwrap();
        // Closed on bad magic, so this reads EOF rather than data. A reset is also fine.
        let mut sink = Vec::new();
        let _ = s.read_to_end(&mut sink).await;
        assert!(sink.is_empty(), "the relay answered a stranger: {sink:?}");
        assert!(waiting.lock().await.is_empty(), "a refusal must not hold a slot");
    }

    /// The self-healing rule, which is why there is no liveness check: an id whose
    /// waiter is already dead must not be wedged forever.
    #[tokio::test]
    async fn a_dead_waiter_does_not_wedge_its_id() {
        let (addr, waiting) = relay().await;
        let dead = dial(addr, ID).await;
        await_waiter(&waiting, ID).await;
        drop(dead); // FIN, and nobody in the relay is reading it to notice

        let mut b = dial(addr, ID).await;
        // Spliced to a corpse: b sees EOF at once instead of hanging on a handshake.
        let mut sink = Vec::new();
        let _ = b.read_to_end(&mut sink).await;
        assert!(sink.is_empty());
        assert!(waiting.lock().await.is_empty(), "the id must be free to redial");
        drop(b);

        // And the redial genuinely works, which is the half that matters.
        let mut c = dial(addr, ID).await;
        await_waiter(&waiting, ID).await;
        let mut d = dial(addr, ID).await;
        c.write_all(b"ok").await.unwrap();
        let mut got = [0u8; 2];
        d.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"ok");
    }

    #[tokio::test]
    async fn distinct_ids_never_cross() {
        let (addr, waiting) = relay().await;
        let mut a1 = dial(addr, ID).await;
        await_waiter(&waiting, ID).await;
        let other = "zzzzzzzzzz012345678901234567890123456789-_x";
        let mut b1 = dial(addr, other).await;
        await_waiter(&waiting, other).await;
        assert_eq!(waiting.lock().await.len(), 2, "two ids, two independent waiters");

        let mut a2 = dial(addr, ID).await;
        a1.write_all(b"for-a").await.unwrap();
        let mut got = [0u8; 5];
        a2.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"for-a");

        let mut b2 = dial(addr, other).await;
        b1.write_all(b"for-b").await.unwrap();
        b2.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"for-b");
    }
}
