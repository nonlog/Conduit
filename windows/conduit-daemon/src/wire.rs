//! Framing, Noise session and identity.
//!
//! The Kotlin side mirrors this byte for byte. The relay needs only [`parse_len`] —
//! it forwards ciphertext and never holds a key.

use anyhow::{bail, Context, Result};
use base64::Engine as _;
use prost::Message as _;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Generated from `proto/conduit.proto` by `build.rs`. The M1 messages
/// (images, notifications) are generated now and used later.
#[allow(dead_code)]
pub mod pb {
    include!(concat!(env!("OUT_DIR"), "/conduit.rs"));
}

/// A Noise transport message cannot exceed 65535 bytes. That protocol limit — not a
/// policy number — is the frame ceiling, so one frame carries exactly one Envelope.
/// (The research notes said 1 MiB; that was never reachable.)
pub const MAX_FRAME: usize = 65535;
/// ChaChaPoly's 16-byte tag comes out of the same budget.
pub const MAX_PLAINTEXT: usize = MAX_FRAME - 16;

pub const NOISE_XX: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
/// Mixed into the handshake hash, so a peer speaking a different dialect fails
/// during the handshake instead of decrypting garbage later.
pub const PROLOGUE: &[u8] = b"conduit/1";

/// The role-aware relay preamble starts with `CDT1`. The next byte is outside base64url so a
/// transition relay can distinguish this 48-byte form from the deployed 47-byte legacy form.
/// Mirrored in `relay/src/main.rs` and Android `Link.kt`.
pub const RELAY_MAGIC: &[u8; 4] = b"CDT1";
const RELAY_INITIATOR: u8 = b'>';
const RELAY_RESPONDER: u8 = b'<';
const RENDEZVOUS_LEN: usize = 43;

/// Parks an outbound connection at the relay, returning it once a partner is spliced in.
///
/// This is the whole of NAT traversal: an outbound TCP connection from each side, with
/// the relay copying ciphertext between them. Nothing arrives on a parked connection
/// until it is paired, so one `peek` is the entire wait — no timer, no poll, and the
/// bytes stay in the socket for the handshake that follows. `Ok(0)` means the relay hung
/// up, which is also what being spliced onto a dead peer looks like from here.
pub async fn park(
    endpoint: &str,
    rendezvous: &str,
    relay_proxy: Option<&str>,
) -> Result<tokio::net::TcpStream> {
    let mut stream = match relay_proxy {
        Some(proxy) => connect_socks5(proxy, endpoint)
            .await
            .with_context(|| format!("dialling relay {endpoint} through {proxy}"))?,
        None => tokio::net::TcpStream::connect(endpoint)
            .await
            .with_context(|| format!("dialling relay {endpoint}"))?,
    };
    stream.set_nodelay(true)?;
    // A parked responder can sit here for hours before a phone arrives. The relay enables
    // keepalive on its half, but that is not enough: if the relay/NAT path dies silently,
    // the server may reap its waiter while Windows keeps this local socket in ESTABLISHED.
    // Without client-side keepalive `peek()` below can then wait forever and `park_forever`
    // never creates the replacement responder, leaving every reconnecting phone waiting by
    // itself. Enable keepalive *before* parking, not only after a partner has already arrived.
    relay_waiter_keepalive(&stream)?;
    let preamble = relay_preamble(rendezvous, RELAY_RESPONDER)?;
    stream.write_all(&preamble).await?;
    stream.flush().await?;

    let mut probe = [0u8; 1];
    if stream.peek(&mut probe).await? == 0 {
        bail!("relay closed the parked connection");
    }
    Ok(stream)
}

async fn connect_socks5(proxy: &str, endpoint: &str) -> Result<tokio::net::TcpStream> {
    let proxy = proxy.strip_prefix("socks5://").unwrap_or(proxy);
    let (host, port) = split_host_port(endpoint)?;
    if host.len() > u8::MAX as usize {
        bail!("SOCKS5 relay hostname is {} bytes, maximum is 255", host.len());
    }

    let mut stream = tokio::net::TcpStream::connect(proxy)
        .await
        .with_context(|| format!("dialling SOCKS5 proxy {proxy}"))?;
    stream.set_nodelay(true)?;

    // RFC 1928: version 5, one method, no authentication.
    stream.write_all(&[5, 1, 0]).await?;
    let mut method = [0u8; 2];
    stream.read_exact(&mut method).await?;
    if method != [5, 0] {
        bail!("SOCKS5 proxy refused no-auth method: {method:?}");
    }

    // Send the hostname, not a locally-resolved IP. Mihomo can then apply DOMAIN rules to the
    // relay even when DNS/fake-IP behaviour differs between networks.
    let mut request = Vec::with_capacity(7 + host.len());
    request.extend_from_slice(&[5, 1, 0, 3, host.len() as u8]);
    request.extend_from_slice(host.as_bytes());
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).await?;

    let mut reply = [0u8; 4];
    stream.read_exact(&mut reply).await?;
    if reply[0] != 5 {
        bail!("SOCKS5 proxy replied with version {}", reply[0]);
    }
    if reply[1] != 0 {
        bail!("SOCKS5 CONNECT to {endpoint} failed with code {}", reply[1]);
    }
    if reply[2] != 0 {
        bail!("SOCKS5 proxy returned a non-zero reserved byte");
    }
    let address_len = match reply[3] {
        1 => 4,
        3 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            len[0] as usize
        }
        4 => 16,
        atyp => bail!("SOCKS5 proxy returned unknown address type {atyp}"),
    };
    let mut discard = vec![0u8; address_len + 2];
    stream.read_exact(&mut discard).await?;
    Ok(stream)
}

fn split_host_port(endpoint: &str) -> Result<(&str, u16)> {
    let (host, port) = endpoint
        .rsplit_once(':')
        .context("relay endpoint has no port")?;
    if host.is_empty() || host.contains(':') {
        bail!("SOCKS5 relay endpoint must be a hostname or IPv4 address plus port");
    }
    let port = port.parse::<u16>().with_context(|| format!("bad relay port {port:?}"))?;
    Ok((host, port))
}

fn relay_waiter_keepalive(stream: &tokio::net::TcpStream) -> Result<()> {
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(30))
        .with_interval(Duration::from_secs(10));
    #[cfg(not(windows))]
    let keepalive = keepalive.with_retries(3);
    socket2::SockRef::from(stream).set_tcp_keepalive(&keepalive)?;
    Ok(())
}

fn relay_preamble(rendezvous: &str, role: u8) -> Result<Vec<u8>> {
    if rendezvous.len() != RENDEZVOUS_LEN || !rendezvous.bytes().all(relay_id_byte) {
        bail!("relay rendezvous id must be {RENDEZVOUS_LEN} base64url bytes");
    }
    if role != RELAY_INITIATOR && role != RELAY_RESPONDER {
        bail!("invalid relay role {role:#04x}");
    }
    let mut preamble = Vec::with_capacity(RELAY_MAGIC.len() + 1 + RENDEZVOUS_LEN);
    preamble.extend_from_slice(RELAY_MAGIC);
    preamble.push(role);
    preamble.extend_from_slice(rendezvous.as_bytes());
    Ok(preamble)
}

fn relay_id_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// Reject an oversize length *before* allocating anything for it.
pub fn parse_len(hdr: [u8; 4]) -> Result<usize> {
    let n = u32::from_be_bytes(hdr) as usize;
    if n == 0 || n > MAX_FRAME {
        bail!("frame length {n} outside 1..={MAX_FRAME}");
    }
    Ok(n)
}

/// Stable peer name: BASE64URL(SHA256(static_pub)), unpadded.
pub fn device_id(static_pub: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(static_pub))
}

/// Short form for the out-of-band comparison during pairing.
pub fn fingerprint(static_pub: &[u8]) -> String {
    Sha256::digest(static_pub)[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// 64 bytes on disk: private ‖ public. Public is stored rather than re-derived
/// because that is one fewer curve operation and one fewer way to be wrong.
pub fn load_or_create_identity(path: &Path) -> Result<snow::Keypair> {
    if let Ok(raw) = std::fs::read(path) {
        if raw.len() == 64 {
            return Ok(snow::Keypair {
                private: raw[..32].to_vec(),
                public: raw[32..].to_vec(),
            });
        }
        bail!("{} is {} bytes, expected 64", path.display(), raw.len());
    }
    let kp = snow::Builder::new(NOISE_XX.parse()?).generate_keypair()?;
    let mut raw = kp.private.clone();
    raw.extend_from_slice(&kp.public);
    std::fs::write(path, &raw).with_context(|| format!("writing {}", path.display()))?;
    Ok(kp)
}

async fn write_framed<W: AsyncWrite + Unpin>(w: &mut W, body: &[u8]) -> Result<()> {
    w.write_all(&(body.len() as u32).to_be_bytes()).await?;
    w.write_all(body).await?;
    w.flush().await?;
    Ok(())
}

/// Read progress, kept in [`Session`] so a cancelled [`Session::recv`] resumes
/// instead of desyncing the stream.
#[derive(Default)]
struct Rx {
    hdr: [u8; 4],
    hdr_got: usize,
    want: usize,
    body_got: usize,
}

pub struct Session {
    noise: snow::TransportState,
    /// Peer's Noise static public key — the thing worth pinning.
    pub peer_static: Vec<u8>,
    rx: Rx,
    /// Receive and send ciphertext must not share storage. [`recv`] is deliberately
    /// cancel-safe: a heartbeat may cancel it after only part of a frame has arrived,
    /// send a PING, and then resume the same receive. If [`send`] overwrites the partial
    /// inbound ciphertext in that gap, the resumed frame fails authentication. Both
    /// buffers are allocated once at [`MAX_FRAME`] and never grow.
    cipher_in: Vec<u8>,
    cipher_out: Vec<u8>,
    plain_in: Vec<u8>,
    plain_out: Vec<u8>,
    next_id: u64,
    /// When this side last put a frame on the wire.
    ///
    /// Lives here rather than in the session loop because every send in the process goes
    /// through [`Session::send`], and a heartbeat driven off a field the caller has to
    /// remember to update is a heartbeat that stops the first time someone adds a
    /// message kind and forgets.
    last_sent: Instant,
}

impl Session {
    /// Noise XX. `initiator` decides who speaks first; both ends use the same code.
    pub async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(
        stream: &mut S,
        local_priv: &[u8],
        initiator: bool,
    ) -> Result<Self> {
        let builder = snow::Builder::new(NOISE_XX.parse()?)
            .prologue(PROLOGUE)?
            .local_private_key(local_priv)?;
        let mut hs = if initiator {
            builder.build_initiator()?
        } else {
            builder.build_responder()?
        };

        let mut buf = vec![0u8; MAX_FRAME];
        let mut discard = vec![0u8; MAX_FRAME];
        let mut my_turn = initiator;
        while !hs.is_handshake_finished() {
            if my_turn {
                let n = hs.write_message(&[], &mut buf)?;
                write_framed(stream, &buf[..n]).await?;
            } else {
                let mut hdr = [0u8; 4];
                stream.read_exact(&mut hdr).await?;
                let n = parse_len(hdr)?;
                stream.read_exact(&mut buf[..n]).await?;
                hs.read_message(&buf[..n], &mut discard)?;
            }
            my_turn = !my_turn;
        }

        let peer_static = hs
            .get_remote_static()
            .context("XX completed without a remote static key")?
            .to_vec();
        Ok(Self {
            noise: hs.into_transport_mode()?,
            peer_static,
            rx: Rx::default(),
            cipher_in: vec![0u8; MAX_FRAME],
            cipher_out: vec![0u8; MAX_FRAME],
            plain_in: vec![0u8; MAX_FRAME],
            plain_out: Vec::with_capacity(4096),
            next_id: 1,
            last_sent: Instant::now(),
        })
    }

    /// How long this side has been silent. The peer's read deadline is the thing this
    /// answers to: it is a promise to speak within a fixed window, and it does not care
    /// in the slightest how much the peer itself has been saying.
    pub fn quiet_for(&self) -> Duration {
        self.last_sent.elapsed()
    }

    /// Not cancel-safe — a half-written frame desyncs the peer. Never wrap this in
    /// `timeout` or `select!`; the write side is driven from one place by design.
    pub async fn send<W: AsyncWrite + Unpin>(
        &mut self,
        w: &mut W,
        kind: pb::Kind,
        payload: &[u8],
    ) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;
        let env = pb::Envelope {
            message_id: id,
            ack_for: 0,
            kind: kind as i32,
            payload: payload.to_vec(),
        };
        let len = env.encoded_len();
        if len > MAX_PLAINTEXT {
            bail!("{kind:?} envelope is {len} B, ceiling is {MAX_PLAINTEXT} B");
        }
        self.plain_out.clear();
        env.encode(&mut self.plain_out)?;
        let n = self.noise.write_message(&self.plain_out, &mut self.cipher_out)?;
        write_framed(w, &self.cipher_out[..n]).await?;
        self.last_sent = Instant::now();
        Ok(id)
    }

    /// Cancel-safe: every await is `read`, and progress lives in `self.rx`, so the
    /// idle timeout in the session loop cannot lose bytes mid-frame.
    pub async fn recv<R: AsyncRead + Unpin>(&mut self, r: &mut R) -> Result<pb::Envelope> {
        while self.rx.hdr_got < 4 {
            let n = r.read(&mut self.rx.hdr[self.rx.hdr_got..]).await?;
            if n == 0 {
                bail!("peer closed");
            }
            self.rx.hdr_got += n;
        }
        if self.rx.want == 0 {
            self.rx.want = parse_len(self.rx.hdr)?;
        }
        while self.rx.body_got < self.rx.want {
            let n = r.read(&mut self.cipher_in[self.rx.body_got..self.rx.want]).await?;
            if n == 0 {
                bail!("peer closed mid-frame");
            }
            self.rx.body_got += n;
        }
        let len = std::mem::take(&mut self.rx).want;
        let m = self.noise.read_message(&self.cipher_in[..len], &mut self.plain_in)?;
        Ok(pb::Envelope::decode(&self.plain_in[..m])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_prefix_bounds_allocation() {
        assert!(parse_len([0, 0, 0, 0]).is_err(), "zero-length frame");
        assert_eq!(parse_len([0, 0, 0, 1]).unwrap(), 1);
        assert_eq!(parse_len((MAX_FRAME as u32).to_be_bytes()).unwrap(), MAX_FRAME);
        assert!(parse_len([0, 1, 0, 0]).is_err(), "65536 must be refused");
        assert!(parse_len([255, 255, 255, 255]).is_err());
    }

    #[test]
    fn device_id_is_stable_and_url_safe() {
        let id = device_id(&[7u8; 32]);
        assert_eq!(id, device_id(&[7u8; 32]));
        assert!(!id.contains('+') && !id.contains('/') && !id.contains('='), "{id}");
        assert_eq!(fingerprint(&[7u8; 32]).split(':').count(), 8);
    }

    #[test]
    fn relay_preamble_carries_the_responder_role_without_ambiguity() {
        let id = device_id(&[7u8; 32]);
        let preamble = relay_preamble(&id, RELAY_RESPONDER).unwrap();
        assert_eq!(preamble.len(), 48);
        assert_eq!(&preamble[..4], RELAY_MAGIC);
        assert_eq!(preamble[4], RELAY_RESPONDER);
        assert_eq!(&preamble[5..], id.as_bytes());
        assert!(!relay_id_byte(RELAY_RESPONDER));
        assert!(relay_preamble("short", RELAY_RESPONDER).is_err());
        assert!(relay_preamble(&id, b'?').is_err());
    }

    #[test]
    fn socks_relay_endpoint_keeps_the_hostname_for_proxy_rules() {
        assert_eq!(split_host_port("tyo.414222.xyz:41113").unwrap(), ("tyo.414222.xyz", 41113));
        assert!(split_host_port("tyo.414222.xyz").is_err());
        assert!(split_host_port("[::1]:41113").is_err());
    }

    #[tokio::test]
    async fn socks5_connect_uses_domain_addressing() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let proxy = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut greeting = [0u8; 3];
            stream.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 1, 0]);
            stream.write_all(&[5, 0]).await.unwrap();

            let mut head = [0u8; 5];
            stream.read_exact(&mut head).await.unwrap();
            assert_eq!(&head[..4], &[5, 1, 0, 3], "CONNECT must carry a domain name");
            let mut host = vec![0u8; head[4] as usize];
            stream.read_exact(&mut host).await.unwrap();
            assert_eq!(host, b"tyo.414222.xyz");
            let mut port = [0u8; 2];
            stream.read_exact(&mut port).await.unwrap();
            assert_eq!(u16::from_be_bytes(port), 41113);

            // Success, IPv4 bind address 127.0.0.1:1234.
            stream
                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0x04, 0xd2])
                .await
                .unwrap();
        });

        let stream = connect_socks5(&format!("socks5://{proxy}"), "tyo.414222.xyz:41113")
            .await
            .unwrap();
        drop(stream);
        server.await.unwrap();
    }

    /// The whole transport: XX handshake, then a text clip each way.
    #[tokio::test]
    async fn xx_handshake_then_clip_both_directions() {
        let (mut a, mut b) = tokio::io::duplex(1 << 16);
        let params: snow::params::NoiseParams = NOISE_XX.parse().unwrap();
        let ka = snow::Builder::new(params.clone()).generate_keypair().unwrap();
        let kb = snow::Builder::new(params).generate_keypair().unwrap();
        let (ka_pub, kb_pub) = (ka.public.clone(), kb.public.clone());

        let init = tokio::spawn(async move {
            let mut s = Session::handshake(&mut a, &ka.private, true).await.unwrap();
            assert_eq!(s.peer_static, kb_pub, "initiator must learn responder's key");
            let clip = pb::ClipText {
                text: "héllo → 世界".into(),
                timestamp_ms: 42,
                mime: "text/plain".into(),
            };
            s.send(&mut a, pb::Kind::ClipText, &clip.encode_to_vec()).await.unwrap();
            let echo = s.recv(&mut a).await.unwrap();
            assert_eq!(echo.kind(), pb::Kind::ClipText);
            pb::ClipText::decode(&echo.payload[..]).unwrap().text
        });

        let mut s = Session::handshake(&mut b, &kb.private, false).await.unwrap();
        assert_eq!(s.peer_static, ka_pub, "responder must learn initiator's key");
        let env = s.recv(&mut b).await.unwrap();
        assert_eq!(env.message_id, 1, "ids start at 1 and are per-session");
        let got = pb::ClipText::decode(&env.payload[..]).unwrap();
        assert_eq!(got.text, "héllo → 世界");
        s.send(&mut b, pb::Kind::ClipText, &env.payload).await.unwrap();

        assert_eq!(init.await.unwrap(), "héllo → 世界");
    }

    /// The regression behind the "session ended after exactly 150 s" churn.
    ///
    /// The desktop's heartbeat used to be a read timeout, so a peer that talked steadily
    /// reset it forever and this side never spoke — while the phone, whose own deadline
    /// can only be met by hearing something, hung up on the dot. Silence has to mean our
    /// silence, so receiving must not reset it and sending must.
    #[tokio::test]
    async fn silence_is_measured_from_our_own_last_send() {
        let (mut a, mut b) = tokio::io::duplex(1 << 16);
        let params: snow::params::NoiseParams = NOISE_XX.parse().unwrap();
        let ka = snow::Builder::new(params.clone()).generate_keypair().unwrap();
        let kb = snow::Builder::new(params).generate_keypair().unwrap();

        let chatter = tokio::spawn(async move {
            let mut s = Session::handshake(&mut a, &ka.private, true).await.unwrap();
            // Two frames, spread out, and never a read: exactly the phone's behaviour.
            for _ in 0..2 {
                s.send(&mut a, pb::Kind::Ping, &[]).await.unwrap();
                tokio::time::sleep(Duration::from_millis(60)).await;
            }
        });

        let mut s = Session::handshake(&mut b, &kb.private, false).await.unwrap();
        s.recv(&mut b).await.unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        s.recv(&mut b).await.unwrap();
        let after_receiving = s.quiet_for();
        assert!(
            after_receiving >= Duration::from_millis(50),
            "receiving reset the heartbeat, which is the bug: {after_receiving:?}"
        );

        s.send(&mut b, pb::Kind::Pong, &[]).await.unwrap();
        assert!(
            s.quiet_for() < after_receiving,
            "sending did not reset the heartbeat"
        );
        chatter.await.unwrap();
    }

    #[tokio::test]
    async fn oversize_envelope_is_refused_before_the_wire() {
        let (mut a, mut b) = tokio::io::duplex(1 << 17);
        let params: snow::params::NoiseParams = NOISE_XX.parse().unwrap();
        let ka = snow::Builder::new(params.clone()).generate_keypair().unwrap();
        let kb = snow::Builder::new(params).generate_keypair().unwrap();
        let resp = tokio::spawn(async move {
            Session::handshake(&mut b, &kb.private, false).await.unwrap();
        });
        let mut s = Session::handshake(&mut a, &ka.private, true).await.unwrap();
        resp.await.unwrap();

        let huge = pb::ClipText {
            text: "x".repeat(MAX_PLAINTEXT),
            ..Default::default()
        };
        let err = s
            .send(&mut a, pb::Kind::ClipText, &huge.encode_to_vec())
            .await
            .expect_err("must refuse, not truncate");
        assert!(err.to_string().contains("ceiling"), "{err}");
    }

    /// Cancelling `recv` mid-frame must not lose bytes — this is the property the
    /// 60 s idle timeout in the session loop depends on.
    #[tokio::test]
    async fn cancelled_recv_resumes() {
        let (mut a, mut b) = tokio::io::duplex(1 << 16);
        let params: snow::params::NoiseParams = NOISE_XX.parse().unwrap();
        let ka = snow::Builder::new(params.clone()).generate_keypair().unwrap();
        let kb = snow::Builder::new(params).generate_keypair().unwrap();
        let sender = tokio::spawn(async move {
            let mut s = Session::handshake(&mut a, &ka.private, true).await.unwrap();
            let clip = pb::ClipText { text: "resumed".into(), ..Default::default() };
            s.send(&mut a, pb::Kind::ClipText, &clip.encode_to_vec()).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        });
        let mut s = Session::handshake(&mut b, &kb.private, false).await.unwrap();

        // Timeouts so short they land inside the frame read, over and over.
        let mut early = None;
        for _ in 0..50 {
            if let Ok(r) = tokio::time::timeout(std::time::Duration::from_nanos(1), s.recv(&mut b)).await
            {
                early = Some(r.unwrap());
                break;
            }
        }
        let env = match early {
            Some(e) => e,
            None => s.recv(&mut b).await.unwrap(),
        };
        assert_eq!(pb::ClipText::decode(&env.payload[..]).unwrap().text, "resumed");
        sender.await.unwrap();
    }

    /// The production regression behind a phone -> desktop file failing at about the relay
    /// heartbeat boundary. A receive may be cancelled with ciphertext already buffered, then
    /// this side is required to send a PING before resuming it. The two directions therefore
    /// cannot reuse one ciphertext scratch buffer.
    #[tokio::test]
    async fn cancelled_recv_survives_an_intervening_send() {
        let (mut a, mut b) = tokio::io::duplex(1 << 16);
        let params: snow::params::NoiseParams = NOISE_XX.parse().unwrap();
        let ka = snow::Builder::new(params.clone()).generate_keypair().unwrap();
        let kb = snow::Builder::new(params).generate_keypair().unwrap();

        let peer = tokio::spawn(async move {
            let session = Session::handshake(&mut a, &ka.private, true).await.unwrap();
            (session, a)
        });
        let mut session = Session::handshake(&mut b, &kb.private, false).await.unwrap();
        let (mut peer, _peer_stream) = peer.await.unwrap();

        // Produce one real encrypted peer frame, but capture it so the test can expose only a
        // prefix to recv before forcing the heartbeat send in between.
        let (mut capture_tx, mut capture_rx) = tokio::io::duplex(1 << 17);
        let body = pb::ClipText {
            text: "x".repeat(32 * 1024),
            ..Default::default()
        };
        peer.send(&mut capture_tx, pb::Kind::ClipText, &body.encode_to_vec())
            .await
            .unwrap();
        drop(capture_tx);
        let mut framed = Vec::new();
        capture_rx.read_to_end(&mut framed).await.unwrap();
        assert!(framed.len() > 32, "test frame must have a body worth interrupting");

        let (mut feed_tx, mut feed_rx) = tokio::io::duplex(1 << 17);
        const PREFIX: usize = 20;
        feed_tx.write_all(&framed[..PREFIX]).await.unwrap();

        let interrupted = tokio::time::timeout(Duration::from_millis(10), session.recv(&mut feed_rx)).await;
        assert!(interrupted.is_err(), "recv unexpectedly completed from a frame prefix");
        assert_eq!(session.rx.hdr_got, 4);
        assert!(session.rx.body_got > 0, "the test did not actually cancel mid-body");

        // This is what serve() does when RELAY_IDLE_PING expires. It must not damage the
        // ciphertext already accumulated by the cancelled receive.
        let mut sink = tokio::io::sink();
        session.send(&mut sink, pb::Kind::Ping, &[]).await.unwrap();

        feed_tx.write_all(&framed[PREFIX..]).await.unwrap();
        let resumed = session.recv(&mut feed_rx).await.unwrap();
        assert_eq!(resumed.kind(), pb::Kind::ClipText);
        assert_eq!(pb::ClipText::decode(&resumed.payload[..]).unwrap().text, body.text);
    }

    /// Cross-language pin for the hand-written Kotlin XX in `android/.../Noise.kt`.
    /// The reference implementation produces the transcript here; `NoiseInteropTest`
    /// replays it byte for byte on the JVM. A handshake that agrees only with itself
    /// is not verified.
    ///
    /// Writes the fixture once if it is missing, then it is a regression test on both
    /// sides: change either implementation and one of the two fails.
    #[test]
    fn noise_xx_fixture_matches_the_reference() {
        // Test-only keys, deliberately recognisable so nobody mistakes them for real ones.
        const IS: [u8; 32] = [0x11; 32];
        const IE: [u8; 32] = [0x22; 32];
        const RS: [u8; 32] = [0x33; 32];
        const RE: [u8; 32] = [0x44; 32];
        const I2R: &[u8] = b"clip:hello";
        const R2I: &[u8] = b"clip:world";

        fn hex(bytes: &[u8]) -> String {
            bytes.iter().map(|b| format!("{b:02x}")).collect()
        }

        let params: snow::params::NoiseParams = NOISE_XX.parse().unwrap();
        let mut ini = snow::Builder::new(params.clone())
            .prologue(PROLOGUE)
            .unwrap()
            .local_private_key(&IS)
            .unwrap()
            .fixed_ephemeral_key_for_testing_only(&IE)
            .build_initiator()
            .unwrap();
        let mut res = snow::Builder::new(params)
            .prologue(PROLOGUE)
            .unwrap()
            .local_private_key(&RS)
            .unwrap()
            .fixed_ephemeral_key_for_testing_only(&RE)
            .build_responder()
            .unwrap();

        let mut buf = vec![0u8; MAX_FRAME];
        let mut sink = vec![0u8; MAX_FRAME];

        // -> e
        let n = ini.write_message(&[], &mut buf).unwrap();
        let msg1 = buf[..n].to_vec();
        res.read_message(&msg1, &mut sink).unwrap();
        // <- e, ee, s, es
        let n = res.write_message(&[], &mut buf).unwrap();
        let msg2 = buf[..n].to_vec();
        ini.read_message(&msg2, &mut sink).unwrap();
        let resp_pub = ini.get_remote_static().unwrap().to_vec();
        // -> s, se
        let n = ini.write_message(&[], &mut buf).unwrap();
        let msg3 = buf[..n].to_vec();
        res.read_message(&msg3, &mut sink).unwrap();
        let init_pub = res.get_remote_static().unwrap().to_vec();

        let mut ini = ini.into_transport_mode().unwrap();
        let mut res = res.into_transport_mode().unwrap();

        // One transport message each way, so the fixture pins the key split and the
        // per-direction nonce order, not just the handshake.
        let n = ini.write_message(I2R, &mut buf).unwrap();
        let ct_i2r = buf[..n].to_vec();
        let m = res.read_message(&ct_i2r, &mut sink).unwrap();
        assert_eq!(&sink[..m], I2R);

        let n = res.write_message(R2I, &mut buf).unwrap();
        let ct_r2i = buf[..n].to_vec();
        let m = ini.read_message(&ct_r2i, &mut sink).unwrap();
        assert_eq!(&sink[..m], R2I);

        let want = format!(
            "# {NOISE_XX}, prologue \"conduit/1\", empty handshake payloads.\n\
             # Generated by conduit-daemon wire::tests from snow; replayed by NoiseInteropTest.\n\
             init_static={}\ninit_static_pub={}\ninit_ephemeral={}\n\
             resp_static={}\nresp_static_pub={}\nresp_ephemeral={}\n\
             init_device_id={}\ninit_fingerprint={}\n\
             msg1={}\nmsg2={}\nmsg3={}\n\
             i2r_plain={}\ni2r={}\nr2i_plain={}\nr2i={}\n",
            hex(&IS),
            hex(&init_pub),
            hex(&IE),
            hex(&RS),
            hex(&resp_pub),
            hex(&RE),
            // Not Noise, but the same class of bug: if the two sides disagree here,
            // pairing fails and the fingerprint on the phone never matches the desktop.
            device_id(&init_pub),
            fingerprint(&init_pub),
            hex(&msg1),
            hex(&msg2),
            hex(&msg3),
            hex(I2R),
            hex(&ct_i2r),
            hex(R2I),
            hex(&ct_r2i),
        );

        let path = Path::new("../../fixtures/noise_xx.txt");
        if !path.exists() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, &want).unwrap();
        }
        let have = std::fs::read_to_string(path).unwrap().replace("\r\n", "\n");
        assert_eq!(have, want, "transcript drifted from the committed fixture");
    }
}
