//! conduit relay — a blind rendezvous for two peers that cannot reach each other.
//!
//! Both ends dial *outbound* TCP here, which is what crosses carrier NAT with no
//! traversal logic whatsoever: no ICE, no STUN, no TURN. Phone Link's 13.5 MB ICE agent
//! is the transport leak this project exists to escape, and this file is the entire
//! replacement for it.
//!
//! The relay is untrusted by construction. Current peers send a 48-byte preamble — `CDT1`,
//! a role, and a rendezvous id that is already public — and then opaque Noise ciphertext.
//! During the protocol transition it also accepts the deployed 47-byte form and infers the
//! old peer's role from whether it immediately starts the Noise handshake. The XX handshake
//! still runs phone-to-desktop *through* here, so session keys never exist on this machine
//! and it cannot read a clipboard or a notification even if it wanted to. There is no config
//! file, no database, and no state that outlives the process: a restart just makes both peers
//! redial.
//!
//! Pairing rule: a connection is spliced to a waiter under the same id and the *opposite*
//! role, and parks otherwise. The role byte is the one thing here that was learned the
//! hard way. It was originally left out — the phone is always the Noise initiator and the
//! desktop always the responder, so the relay appeared not to need it — but that reasoning
//! only holds while every waiter is live. A phone whose session died leaves its socket
//! parked, unnoticed because nothing here reads it; the same phone's next attempt then
//! presents the same id and gets spliced to its own corpse. Two initiators, each handed
//! the other's 32-byte first message where an 80-byte second one was due, on a 300 s retry
//! loop. With a role, that arrival displaces the stale waiter instead of marrying it.
//!
//! Staleness still needs no detection. A waiter whose TCP died is spliced to the next
//! arrival of the opposite role, the copy ends at once, the live peer sees EOF and
//! redials. One wasted round trip instead of a liveness probe. Waiters that are merely
//! idle — a desktop waiting hours for a phone — are the correct behaviour, and the
//! kernel's own keepalive both reaps the genuinely dead ones and refreshes the NAT mapping
//! that keeps the live ones reachable. The only userspace timeout after the preamble is the
//! short, migration-only legacy-role inference window; explicit-role peers never pay it.

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

/// Which half of the Noise handshake this peer intends to be: the phone speaks first, the
/// desktop waits. Deliberately outside the base64url alphabet, so a peer still speaking
/// the 47-byte preamble fails the check below with a message that says what is wrong
/// rather than stalling until the deadline on a byte that never comes.
const INITIATOR: u8 = b'>';
const RESPONDER: u8 = b'<';

/// `BASE64URL(SHA256(static_pub))` unpadded is always 43 characters, so the preamble is
/// a fixed size: no length field, no delimiter, no parser to get wrong.
const ID_LEN: usize = 43;

/// Magic and role. Read before the id so an unknown role is refused immediately.
const HEAD: usize = MAGIC.len() + 1;

/// A connection that has not sent its preamble is holding an fd for nothing, and the
/// kernel's keepalive cannot help — such a peer is alive, merely silent.
const PREAMBLE_DEADLINE: Duration = Duration::from_secs(10);

/// The deployed 47-byte preamble has no role. Its initiator writes Noise message 1
/// immediately after the id, while its responder writes nothing until a partner speaks.
/// Waiting one second here is a migration tax only: it is long compared with the same task's
/// next write, but tiny compared with the relay's normal multi-minute idle cadence.
const LEGACY_ROLE_GRACE: Duration = Duration::from_secs(1);

/// One fd per waiter, so this ceiling bounds a buggy or hostile client rather than
/// rationing a real resource. Far past any plausible number of paired devices.
const MAX_WAITING: usize = 256;

const DEFAULT_PORT: u16 = 41113;

/// Rendezvous id and role. Keyed by both, because at most one waiter per role per id is
/// exactly what stops a peer from being spliced to a stale copy of itself.
type Key = (String, u8);

type Waiting = Mutex<HashMap<Key, TcpStream>>;

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

    let (id, role, legacy) = read_preamble(&mut stream).await?;
    let want = match role {
        INITIATOR => RESPONDER,
        RESPONDER => INITIATOR,
        other => bail!(
            "unknown role {:?}, expected an initiator or a responder",
            other as char
        ),
    };

    let mut waiter = {
        let mut map = waiting.lock().await;
        match map.remove(&(id.clone(), want)) {
            Some(waiter) => waiter,
            None => {
                if map.len() >= MAX_WAITING {
                    bail!("{} peers already waiting, refusing this one", map.len());
                }
                // Displaces any earlier waiter of this same role, which is the repair for a
                // peer that reconnected without its previous socket being noticed as dead.
                // Dropping it here is the correct outcome either way: if it really was dead
                // this costs nothing, and if the peer is somehow live it redials — where
                // leaving it would have wedged the id until one end restarted.
                if let Some(stale) = map.insert((id.clone(), role), stream) {
                    info!(id = %short(&id), role = %role as char, "displaced a stale waiter");
                    drop(stale);
                }
                info!(
                    %peer,
                    id = %short(&id),
                    role = %role as char,
                    legacy,
                    waiting = map.len(),
                    "waiting for a partner"
                );
                return Ok(());
            }
        }
    };

    let n = pairs.fetch_add(1, Ordering::Relaxed) + 1;
    info!(%peer, id = %short(&id), role = %role as char, legacy, pair = n, "spliced");
    // The only thing this process ever does with a payload. No parse, no inspection, no
    // buffer of its own beyond the two `copy_bidirectional` allocates.
    match tokio::io::copy_bidirectional(&mut waiter, &mut stream).await {
        Ok((up, down)) => info!(id = %short(&id), up, down, "pair closed"),
        // Expected, not exceptional: one side going away is how every session ends.
        Err(e) => info!(id = %short(&id), error = %e, "pair closed"),
    }
    Ok(())
}

/// Reads either the current role-aware preamble or the deployed legacy one without consuming
/// any Noise byte. Role markers deliberately sit outside the base64url alphabet, so byte five
/// is an unambiguous discriminator and no version field is needed for this transition.
async fn read_preamble(stream: &mut TcpStream) -> Result<(String, u8, bool)> {
    let mut head = [0u8; HEAD];
    tokio::time::timeout(PREAMBLE_DEADLINE, stream.read_exact(&mut head))
        .await
        .context("no preamble before the deadline")??;
    if head[..MAGIC.len()] != MAGIC {
        bail!("bad magic, not a conduit peer");
    }

    let marker = head[MAGIC.len()];
    let (role, id, legacy) = match marker {
        INITIATOR | RESPONDER => {
            let mut id = [0u8; ID_LEN];
            read_deadlined(stream, &mut id, "no rendezvous id before the deadline").await?;
            (marker, id, false)
        }
        first if id_byte(first) => {
            // Legacy is `CDT1 + id`: the byte already read as `marker` is id[0]. Read only
            // the remaining 42 bytes, then peek so the first Noise byte stays in the socket.
            let mut id = [0u8; ID_LEN];
            id[0] = first;
            read_deadlined(
                stream,
                &mut id[1..],
                "no legacy rendezvous id before the deadline",
            )
            .await?;
            let role = infer_legacy_role(stream).await?;
            (role, id, true)
        }
        other => bail!("unknown relay role/version marker {other:#04x}"),
    };

    if !id.iter().copied().all(id_byte) {
        bail!("rendezvous id is not base64url");
    }
    let id = std::str::from_utf8(&id)
        .context("rendezvous id is not utf-8")?
        .to_owned();
    Ok((id, role, legacy))
}

async fn read_deadlined(stream: &mut TcpStream, buf: &mut [u8], what: &'static str) -> Result<()> {
    tokio::time::timeout(PREAMBLE_DEADLINE, stream.read_exact(buf))
        .await
        .context(what)??;
    Ok(())
}

async fn infer_legacy_role(stream: &TcpStream) -> Result<u8> {
    let mut probe = [0u8; 1];
    match tokio::time::timeout(LEGACY_ROLE_GRACE, stream.peek(&mut probe)).await {
        Ok(Ok(0)) => bail!("legacy peer closed before its role could be inferred"),
        Ok(Ok(_)) => Ok(INITIATOR),
        Ok(Err(e)) => Err(e).context("peeking legacy peer after the preamble"),
        Err(_) => Ok(RESPONDER),
    }
}

fn id_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
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
        dial_as(addr, id, INITIATOR).await
    }

    async fn dial_as(addr: SocketAddr, id: &str, role: u8) -> TcpStream {
        // A wrong-length id would stall on the fixed-size preamble and surface as
        // "never registered the waiter" ten seconds later, which reads like a relay bug.
        assert_eq!(id.len(), ID_LEN, "test id must be a realistic length");
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(&MAGIC).await.unwrap();
        s.write_all(&[role]).await.unwrap();
        s.write_all(id.as_bytes()).await.unwrap();
        s
    }

    async fn dial_legacy(addr: SocketAddr, id: &str) -> TcpStream {
        assert_eq!(id.len(), ID_LEN, "test id must be a realistic length");
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(&MAGIC).await.unwrap();
        s.write_all(id.as_bytes()).await.unwrap();
        s
    }

    /// A deployed phone starts Noise immediately after its 47-byte preamble. These bytes are
    /// deliberately arbitrary: role inference only peeks and must leave them untouched.
    async fn dial_legacy_initiator(addr: SocketAddr, id: &str, first_noise: &[u8]) -> TcpStream {
        let mut s = dial_legacy(addr, id).await;
        s.write_all(first_noise).await.unwrap();
        s
    }

    /// Accepting is asynchronous, so a test dialling twice in a microsecond could leave
    /// both peers as waiters that never meet. Real peers are minutes apart; this is the
    /// only place the ordering has to be forced.
    async fn await_waiter(waiting: &Waiting, id: &str) {
        await_waiter_as(waiting, id, INITIATOR).await
    }

    async fn await_waiter_as(waiting: &Waiting, id: &str, role: u8) {
        // A legacy responder is deliberately classified only after a one-second quiet window.
        for _ in 0..2500 {
            if waiting.lock().await.contains_key(&(id.to_owned(), role)) {
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
        let mut b = dial_as(addr, ID, RESPONDER).await;

        let mut got = [0u8; 10];
        b.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"noise-msg1", "the relay altered the payload");

        b.write_all(b"noise-msg2!").await.unwrap();
        let mut back = [0u8; 11];
        a.read_exact(&mut back).await.unwrap();
        assert_eq!(&back, b"noise-msg2!", "the return direction is not spliced");
        assert!(
            waiting.lock().await.is_empty(),
            "a spliced pair must leave the map"
        );
    }

    #[tokio::test]
    async fn a_stranger_is_refused_and_occupies_nothing() {
        let (addr, waiting) = relay().await;
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(b"HTTP").await.unwrap();
        s.write_all(&[INITIATOR]).await.unwrap();
        s.write_all(&[b'x'; ID_LEN]).await.unwrap();
        // Closed on bad magic, so this reads EOF rather than data. A reset is also fine.
        let mut sink = Vec::new();
        let _ = s.read_to_end(&mut sink).await;
        assert!(sink.is_empty(), "the relay answered a stranger: {sink:?}");
        assert!(
            waiting.lock().await.is_empty(),
            "a refusal must not hold a slot"
        );
    }

    /// The bug this role byte exists for: a phone whose session died leaves a socket parked
    /// that nothing here is reading, so its own next attempt used to be spliced to it. Two
    /// initiators, each handed the other's 32-byte first Noise message where an 80-byte
    /// second one was due — which is an IndexOutOfBoundsException on the phone and a link
    /// down for hours on a 300 s retry loop.
    #[tokio::test]
    async fn a_peer_is_never_spliced_to_a_stale_copy_of_itself() {
        let (addr, waiting) = relay().await;
        let mut dead = dial(addr, ID).await;
        await_waiter(&waiting, ID).await;

        // The same side again, as happens on every reconnect. It must park, not pair.
        let mut again = dial(addr, ID).await;
        await_waiter(&waiting, ID).await;
        assert_eq!(
            waiting.lock().await.len(),
            1,
            "one waiter per role, the stale one gone"
        );
        // And the displaced socket is closed rather than left holding an fd forever.
        let mut sink = Vec::new();
        let _ = dead.read_to_end(&mut sink).await;
        assert!(sink.is_empty(), "the stale waiter should have been dropped");

        // The reconnect then meets the real partner, which is the half that matters.
        let mut desktop = dial_as(addr, ID, RESPONDER).await;
        again.write_all(b"msg1").await.unwrap();
        let mut got = [0u8; 4];
        desktop.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"msg1");
    }

    #[tokio::test]
    async fn two_legacy_peers_are_classified_and_spliced_without_consuming_noise() {
        let (addr, waiting) = relay().await;
        let mut phone = dial_legacy_initiator(addr, ID, b"old-noise1").await;
        await_waiter_as(&waiting, ID, INITIATOR).await;
        let mut desktop = dial_legacy(addr, ID).await;

        let mut got = [0u8; 10];
        desktop.read_exact(&mut got).await.unwrap();
        assert_eq!(
            &got, b"old-noise1",
            "legacy role inference consumed Noise bytes"
        );
        desktop.write_all(b"old-noise2").await.unwrap();
        phone.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"old-noise2");
        assert!(waiting.lock().await.is_empty());
    }

    #[tokio::test]
    async fn explicit_and_legacy_peers_interoperate_in_both_upgrade_orders() {
        let (addr, waiting) = relay().await;

        // Desktop upgraded first, phone still on the deployed 47-byte form.
        let mut new_desktop = dial_as(addr, ID, RESPONDER).await;
        await_waiter_as(&waiting, ID, RESPONDER).await;
        let old_phone = dial_legacy_initiator(addr, ID, b"phone-old").await;
        let mut got = [0u8; 9];
        new_desktop.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"phone-old");
        drop(old_phone);
        drop(new_desktop);

        // Phone upgraded first, desktop still on the deployed 47-byte form.
        let mut new_phone = dial(addr, ID).await;
        new_phone.write_all(b"phone-new").await.unwrap();
        await_waiter_as(&waiting, ID, INITIATOR).await;
        let mut old_desktop = dial_legacy(addr, ID).await;
        old_desktop.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"phone-new");
    }

    #[tokio::test]
    async fn a_legacy_phone_reconnect_displaces_its_stale_copy() {
        let (addr, waiting) = relay().await;
        let mut stale = dial_legacy_initiator(addr, ID, b"stale").await;
        await_waiter_as(&waiting, ID, INITIATOR).await;

        let mut fresh = dial_legacy_initiator(addr, ID, b"fresh").await;
        await_waiter_as(&waiting, ID, INITIATOR).await;
        assert_eq!(waiting.lock().await.len(), 1);

        let mut sink = Vec::new();
        let _ = stale.read_to_end(&mut sink).await;
        assert!(sink.is_empty(), "legacy stale waiter was not displaced");

        let mut desktop = dial_as(addr, ID, RESPONDER).await;
        let mut got = [0u8; 5];
        desktop.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"fresh");
        desktop.write_all(b"reply").await.unwrap();
        fresh.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"reply");
    }

    /// The self-healing rule, which is why there is no liveness check: an id whose
    /// waiter is already dead must not be wedged forever.
    #[tokio::test]
    async fn a_dead_waiter_does_not_wedge_its_id() {
        let (addr, waiting) = relay().await;
        let dead = dial(addr, ID).await;
        await_waiter(&waiting, ID).await;
        drop(dead); // FIN, and nobody in the relay is reading it to notice

        let mut b = dial_as(addr, ID, RESPONDER).await;
        // Spliced to a corpse: b sees EOF at once instead of hanging on a handshake.
        let mut sink = Vec::new();
        let _ = b.read_to_end(&mut sink).await;
        assert!(sink.is_empty());
        assert!(
            waiting.lock().await.is_empty(),
            "the id must be free to redial"
        );
        drop(b);

        // And the redial genuinely works, which is the half that matters.
        let mut c = dial(addr, ID).await;
        await_waiter(&waiting, ID).await;
        let mut d = dial_as(addr, ID, RESPONDER).await;
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
        assert_eq!(
            waiting.lock().await.len(),
            2,
            "two ids, two independent waiters"
        );

        let mut a2 = dial_as(addr, ID, RESPONDER).await;
        a1.write_all(b"for-a").await.unwrap();
        let mut got = [0u8; 5];
        a2.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"for-a");

        let mut b2 = dial_as(addr, other, RESPONDER).await;
        b1.write_all(b"for-b").await.unwrap();
        b2.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"for-b");
    }

    /// Opposite roles under the same id are partners, not two independent waiting slots.
    #[tokio::test]
    async fn opposite_roles_of_one_id_splice_immediately() {
        let (addr, waiting) = relay().await;
        let mut phone = dial(addr, ID).await;
        phone.write_all(b"hello").await.unwrap();
        await_waiter_as(&waiting, ID, INITIATOR).await;
        let mut desktop = dial_as(addr, ID, RESPONDER).await;
        let mut got = [0u8; 5];
        desktop.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"hello");
        assert!(waiting.lock().await.is_empty());
    }
}
