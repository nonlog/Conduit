//! Framing, Noise session and identity.
//!
//! The Kotlin side mirrors this byte for byte. The relay needs only [`parse_len`] —
//! it forwards ciphertext and never holds a key.

use anyhow::{bail, Context, Result};
use base64::Engine as _;
use prost::Message as _;
use sha2::{Digest, Sha256};
use std::path::Path;
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
    /// All three are allocated once at [`MAX_FRAME`] and never grow, which is what
    /// makes per-session memory a constant instead of a function of traffic.
    cipher: Vec<u8>,
    plain_in: Vec<u8>,
    plain_out: Vec<u8>,
    next_id: u64,
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
            cipher: vec![0u8; MAX_FRAME],
            plain_in: vec![0u8; MAX_FRAME],
            plain_out: Vec::with_capacity(4096),
            next_id: 1,
        })
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
        let n = self.noise.write_message(&self.plain_out, &mut self.cipher)?;
        write_framed(w, &self.cipher[..n]).await?;
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
            let n = r.read(&mut self.cipher[self.rx.body_got..self.rx.want]).await?;
            if n == 0 {
                bail!("peer closed mid-frame");
            }
            self.rx.body_got += n;
        }
        let len = std::mem::take(&mut self.rx).want;
        let m = self.noise.read_message(&self.cipher[..len], &mut self.plain_in)?;
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
}
