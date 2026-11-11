//! Bounded connection admission before touching the existing authenticated session.
//!
//! A TCP accept (including a port probe) is not a reconnect. Noise and the identity
//! hello are completed on the candidate's own stream; pairing policy is rechecked
//! by main immediately before promotion. No clipboard, transfer or trust state is
//! changed here. Timing out discards the entire candidate, never a partial frame
//! from a session that will be reused.

use crate::wire::{self, pb, Session};
use anyhow::{bail, Result};
use prost::Message as _;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

pub const MAX_PENDING: usize = 4;
pub const TIMEOUT: Duration = Duration::from_secs(12);

pub struct Negotiated {
    pub session: Session,
    pub remote_id: String,
    pub hello: pb::PairRequest,
}

/// One generation for the currently admitted connection/pairing policy. Once a
/// candidate wins (or the user changes trust), older in-flight candidates cannot
/// displace it. The main event loop owns this value, so there is no extra worker.
#[derive(Default)]
pub struct Generation(u64);

impl Generation {
    pub fn ticket(&self) -> u64 {
        self.0
    }
    pub fn accepts(&self, ticket: u64) -> bool {
        self.0 == ticket
    }
    pub fn invalidate(&mut self) {
        self.0 = self.0.wrapping_add(1);
    }
}

/// One Noise XX handshake plus the mutual identity hello, on the candidate's own stream.
///
/// The hello is built after the handshake because it may need the authenticated peer id — a
/// pairing proof binds both device ids, and the initiator only learns the responder's id once the
/// handshake has completed. `initiator` decides who speaks first on the wire.
pub async fn negotiate_with<S, F>(
    stream: &mut S,
    private_key: &[u8],
    initiator: bool,
    hello_for: F,
) -> Result<Negotiated>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnOnce(&str) -> pb::PairRequest,
{
    let mut session = Session::handshake(stream, private_key, initiator).await?;
    let remote_id = wire::device_id(&session.peer_static);
    let hello = hello_for(&remote_id);
    session
        .send(stream, pb::Kind::PairRequest, &hello.encode_to_vec())
        .await?;
    let envelope = session.recv(stream).await?;
    if envelope.kind() != pb::Kind::PairRequest {
        bail!("peer did not send its authenticated identity hello");
    }
    let peer = pb::PairRequest::decode(&envelope.payload[..])?;
    if !peer.device_id.is_empty() && peer.device_id != remote_id {
        bail!("announced peer id does not match its Noise static key");
    }
    Ok(Negotiated {
        session,
        remote_id,
        hello: peer,
    })
}

/// Inbound form: this side answers, and the hello does not depend on the peer's id. Kept as the
/// convenience wrapper for callers that do not need the authenticated id while building it.
#[allow(dead_code)]
pub async fn negotiate<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    private_key: &[u8],
    hello: &pb::PairRequest,
) -> Result<Negotiated> {
    negotiate_with(stream, private_key, false, |_| hello.clone()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt as _;

    fn key() -> snow::Keypair {
        snow::Builder::new(wire::NOISE_XX.parse().unwrap())
            .generate_keypair()
            .unwrap()
    }

    #[test]
    fn older_candidate_cannot_replace_a_new_session_or_changed_pairing_policy() {
        let mut generation = Generation::default();
        let first = generation.ticket();
        let concurrent = generation.ticket();
        assert!(generation.accepts(first));
        generation.invalidate();
        assert!(!generation.accepts(concurrent));
        let before_forget = generation.ticket();
        generation.invalidate();
        assert!(!generation.accepts(before_forget));
        assert!(generation.accepts(generation.ticket()));
    }

    #[tokio::test]
    async fn connect_close_and_malformed_probe_never_complete_admission() {
        let local = key();
        let hello = pb::PairRequest::default();
        let (mut stream, peer) = tokio::io::duplex(64);
        drop(peer);
        assert!(negotiate(&mut stream, &local.private, &hello)
            .await
            .is_err());
        let (mut stream, mut peer) = tokio::io::duplex(64);
        peer.write_all(&u32::MAX.to_be_bytes()).await.unwrap();
        assert!(negotiate(&mut stream, &local.private, &hello)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn stalled_candidate_is_bounded_and_does_not_advance_generation() {
        let generation = Generation::default();
        let ticket = generation.ticket();
        let local = key();
        let (mut stream, _silent_peer) = tokio::io::duplex(64);
        assert!(tokio::time::timeout(
            Duration::from_millis(25),
            negotiate(&mut stream, &local.private, &pb::PairRequest::default())
        )
        .await
        .is_err());
        assert!(generation.accepts(ticket));
    }

    #[tokio::test]
    async fn initiator_side_negotiates_and_sees_the_peer_id_before_its_hello() {
        for initiator in [true, false] {
            let local = key();
            let peer = key();
            let local_id = wire::device_id(&local.public);
            let peer_id = wire::device_id(&peer.public);
            let (mut stream, mut other) = tokio::io::duplex(4096);
            let peer_id_for_task = peer_id.clone();
            let local_id_for_task = local_id.clone();
            let peer_task = tokio::spawn(async move {
                negotiate_with(&mut other, &peer.private, !initiator, |remote_id: &str| {
                    assert_eq!(remote_id, local_id_for_task);
                    pb::PairRequest {
                        device_id: peer_id_for_task,
                        transfer_protocol: 1,
                        pairing_proof: b"proof built from the peer id".to_vec(),
                        ..Default::default()
                    }
                })
                .await
                .unwrap()
            });
            let seen = peer_id.clone();
            let candidate = negotiate_with(&mut stream, &local.private, initiator, |remote_id| {
                assert_eq!(remote_id, seen);
                pb::PairRequest {
                    device_id: wire::device_id(&local.public),
                    transfer_protocol: 1,
                    ..Default::default()
                }
            })
            .await
            .unwrap();
            assert_eq!(candidate.remote_id, peer_id);
            assert_eq!(candidate.hello.transfer_protocol, 1);
            assert_eq!(
                candidate.hello.pairing_proof,
                b"proof built from the peer id"
            );
            let other = peer_task.await.unwrap();
            assert_eq!(other.remote_id, local_id);
            assert_eq!(other.hello.device_id, local_id);
        }
    }

    #[tokio::test]
    async fn cryptographic_peer_and_announced_id_must_agree() {
        for matching in [true, false] {
            let desktop = key();
            let phone = key();
            let id = wire::device_id(&phone.public);
            let announced = if matching {
                id.clone()
            } else {
                "unrelated-peer".into()
            };
            let (mut server, mut client) = tokio::io::duplex(4096);
            let client_task = tokio::spawn(async move {
                let mut session = Session::handshake(&mut client, &phone.private, true)
                    .await
                    .unwrap();
                let remote_hello = session.recv(&mut client).await.unwrap();
                assert_eq!(remote_hello.kind(), pb::Kind::PairRequest);
                let hello = pb::PairRequest {
                    device_id: announced,
                    transfer_protocol: 1,
                    ..Default::default()
                };
                session
                    .send(&mut client, pb::Kind::PairRequest, &hello.encode_to_vec())
                    .await
                    .unwrap();
            });
            let hello = pb::PairRequest {
                device_id: wire::device_id(&desktop.public),
                ..Default::default()
            };
            let candidate = negotiate(&mut server, &desktop.private, &hello).await;
            if matching {
                let candidate = candidate.unwrap();
                assert_eq!(candidate.remote_id, id);
                assert_eq!(candidate.hello.transfer_protocol, 1);
            } else {
                assert!(candidate.is_err());
            }
            client_task.await.unwrap();
        }
    }
}
