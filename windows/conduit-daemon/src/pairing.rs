//! Multi-peer trust and explicit pairing state for the Windows companion.
//!
//! One data directory holds up to [`MAX_PEERS`] trusted devices: the pinned phone and, optionally,
//! other desktops on the same LAN or Relay. `paired-peers.tsv` is authoritative; `peer.txt` and
//! `peer-name.txt` are kept only as derived compatibility files for older tooling. A pairing
//! window is still a short user-opened interval, but both LAN discovery and the temporary
//! Relay rendezvous derived from the human pairing code may use it, and a desktop-to-desktop
//! pair is bound to the code with [`pairing_proof`] so a relay cannot splice two unrelated pairs.

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use sha2::{Digest, Sha256};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::wire::pb;

pub const MAX_PEERS: usize = 8;

const PEER_FILE: &str = "peer.txt";
const PEER_NAME_FILE: &str = "peer-name.txt";
const PEERS_FILE: &str = "paired-peers.tsv";
const SELECTED_FILE: &str = "selected-peer.txt";
/// Locally forgotten phone-sourced desktops. A roster cannot resurrect a device the user removed
/// here until the phone drops it and lists it again.
const DISMISSED_FILE: &str = "dismissed-peers.txt";
const PAIRING_FILE: &str = "pairing.txt";
/// Marks data directories that have already entered the explicit-pairing model. Without this, an
/// offline phone could resurrect itself through the one-time legacy Relay migration after the user
/// explicitly chose Forget.
const PAIRING_MODEL_FILE: &str = "pairing-v2";
/// Marks a directory whose authoritative registry is `paired-peers.tsv`, so the legacy
/// `peer.txt` import happens exactly once.
const PEERS_MODEL_FILE: &str = "peers-v3";
/// Derived multi-peer status snapshot (written by [`crate::status`]); cleared with the rest of the
/// pairing state when a cloned installation is re-keyed.
const STATUS_PEERS_FILE: &str = "peers.tsv";
const ID_LEN: usize = 43;
const CODE_LEN: usize = 6;
const PAIRING_DOMAIN: &[u8] = b"conduit-pair-v2:";
const PROOF_DOMAIN: &str = "conduit-pair-proof-v1";
/// Wrong pairing proofs a single opening of the window tolerates before it closes itself.
const MAX_ATTEMPTS: u32 = 5;
const MAX_DISMISSED: usize = 64;
pub const WINDOW: Duration = Duration::from_secs(120);

/// Serializes every read-modify-write of the registry, the tombstones and the pairing window in
/// this process. The main loop and the session tasks both write these files, and a lost update
/// here is a lost trust decision; the files themselves are the shared state, not this lock.
static REGISTRY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock_registry() -> std::sync::MutexGuard<'static, ()> {
    // A panic elsewhere must not brick pairing, so poisoning is deliberately ignored.
    REGISTRY_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerKind {
    Phone,
    Desktop,
}

impl PeerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Phone => "phone",
            Self::Desktop => "desktop",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "phone" => Some(Self::Phone),
            "desktop" => Some(Self::Desktop),
            _ => None,
        }
    }

    /// The form factor a peer announced. Anything but an explicit desktop is a phone, which is
    /// the conservative default: a phone kind grants no desktop-only privilege.
    pub fn announced(device_kind: i32) -> Self {
        if device_kind == pb::DeviceKind::Desktop as i32 {
            Self::Desktop
        } else {
            Self::Phone
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerSource {
    Direct,
    Phone,
}

impl PeerSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Phone => "phone",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "direct" => Some(Self::Direct),
            "phone" => Some(Self::Phone),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PairedPeer {
    pub id: String,
    pub name: String,
    pub kind: PeerKind,
    pub enabled: bool,
    pub source: PeerSource,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct State {
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub pairing: bool,
    pub pairing_code: Option<String>,
    pub pairing_rendezvous: Option<String>,
    pub pairing_expires_ms: Option<u128>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PairingWindow {
    pub code: String,
    pub rendezvous: String,
    pub expires_ms: u128,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Authorization {
    Trusted,
    Paired,
    LegacyRelayMigration,
}

pub fn state(dir: &Path) -> State {
    let window = pairing_window(dir);
    State {
        device_id: peer_id(dir),
        device_name: peer_name(dir),
        pairing: window.is_some(),
        pairing_code: window.as_ref().map(|value| value.code.clone()),
        pairing_rendezvous: window.as_ref().map(|value| value.rendezvous.clone()),
        pairing_expires_ms: window.map(|value| value.expires_ms),
    }
}

/// The pinned phone, derived compatibility file included.
pub fn peer_id(dir: &Path) -> Option<String> {
    phone_peer(dir).map(|peer| peer.id)
}

pub fn peer_name(dir: &Path) -> Option<String> {
    phone_peer(dir)
        .map(|peer| peer.name)
        .filter(|name| !name.is_empty())
}

fn phone_peer(dir: &Path) -> Option<PairedPeer> {
    let peers = peers(dir);
    peers
        .iter()
        .find(|peer| Some(peer.id.as_str()) == selected_peer(dir).as_deref())
        .filter(|peer| peer.kind == PeerKind::Phone)
        .or_else(|| peers.iter().find(|peer| peer.kind == PeerKind::Phone))
        .cloned()
}

pub fn start(dir: &Path) -> Result<PairingWindow> {
    let _guard = lock_registry();
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let code = generate_code()?;
    let expires_ms = now_ms().saturating_add(WINDOW.as_millis());
    atomic_write(
        &dir.join(PAIRING_FILE),
        &format!("expires_ms={expires_ms}\ncode={code}\nattempts=0\n"),
    )?;
    Ok(PairingWindow {
        rendezvous: pairing_rendezvous(&code),
        code,
        expires_ms,
    })
}

/// Opens this side's window with a code entered for another device, so both ends derive the same
/// rendezvous and the same proof. Used by `pair join <code>`.
pub fn join(dir: &Path, code: &str) -> Result<PairingWindow> {
    let _guard = lock_registry();
    let code = normalize_code(code);
    if code.len() != CODE_LEN {
        bail!("a pairing code is exactly {CODE_LEN} digits");
    }
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let expires_ms = now_ms().saturating_add(WINDOW.as_millis());
    atomic_write(
        &dir.join(PAIRING_FILE),
        &format!("expires_ms={expires_ms}\ncode={code}\nattempts=0\n"),
    )?;
    Ok(PairingWindow {
        rendezvous: pairing_rendezvous(&code),
        code,
        expires_ms,
    })
}

pub fn cancel(dir: &Path) -> Result<bool> {
    let _guard = lock_registry();
    remove_if_exists(&dir.join(PAIRING_FILE))
}

/// Forgets every paired device. The identity file is not pairing state and stays.
pub fn forget(dir: &Path) -> Result<bool> {
    let _guard = lock_registry();
    let mut changed = false;
    for file in [
        PEER_FILE,
        PEER_NAME_FILE,
        PEERS_FILE,
        SELECTED_FILE,
        DISMISSED_FILE,
        STATUS_PEERS_FILE,
        PAIRING_FILE,
    ] {
        changed |= remove_if_exists(&dir.join(file))?;
    }
    mark_explicit_pairing_model(dir)?;
    // An explicit forget must also retire the legacy Relay migration and the one-time import.
    atomic_write(&dir.join(PEERS_MODEL_FILE), "1\n")?;
    Ok(changed)
}

pub fn remember_name(dir: &Path, id: &str, name: &str) -> Result<()> {
    let name = clean_name(name);
    if name.is_empty() {
        return Ok(());
    }
    let _guard = lock_registry();
    let mut peers = peers_locked(dir);
    let Some(peer) = peers.iter_mut().find(|peer| peer.id == id) else {
        return Ok(());
    };
    if peer.name == name {
        return Ok(());
    }
    peer.name = name;
    write_peers_locked(dir, &peers)
}

/// Decides whether one completed Noise XX handshake may become an application session.
///
/// A device that is not in `paired-peers.tsv` is accepted only while the user has explicitly
/// opened pairing on both sides. LAN discovery and the temporary pairing-code Relay rendezvous
/// are both acceptable pairing transports; the normal long-lived Relay rendezvous never is.
/// The announced form factor decides which privileges the session gets, so a phone cannot claim
/// the desktop role and a desktop cannot join the PC-to-PC mesh without proving the code.
/// `initiator` is true when this side dialled, which is what fixes the id order of the proof.
pub fn authorize_candidate(
    dir: &Path,
    local_id: &str,
    remote_id: &str,
    via: &str,
    initiator: bool,
    hello: &pb::PairRequest,
) -> Result<(Authorization, PeerKind)> {
    if !valid_id(remote_id) || !valid_id(local_id) {
        bail!("peer id is not a valid Conduit device id");
    }
    if remote_id == local_id {
        bail!("a device cannot pair with itself");
    }
    let peers = peers(dir);
    let known = peers.iter().find(|peer| peer.id == remote_id).cloned();
    let announced = PeerKind::announced(hello.device_kind);
    // An already trusted device keeps the kind this computer accepted it as: an open pairing
    // window must never let a paired desktop re-announce itself as the phone.
    let kind = known.as_ref().map_or(announced, |peer| peer.kind);

    let Some((window, _)) = window_with_attempts(dir) else {
        let Some(known) = known else {
            // Upgrade compatibility only: old Windows builds had no peer store at all, while an
            // already paired phone knows this desktop's unguessable rendezvous id and can therefore
            // reach this exact relay parking slot. Pin that phone after the mutual hello succeeds.
            if peers.is_empty()
                && via == "relay"
                && !dir.join(PAIRING_MODEL_FILE).exists()
                && !dir.join(PEERS_MODEL_FILE).exists()
            {
                return Ok((Authorization::LegacyRelayMigration, PeerKind::Phone));
            }
            bail!("peer is not paired; open pairing on both devices first");
        };
        if !known.enabled {
            bail!("this paired device is disabled on this computer");
        }
        return Ok((Authorization::Trusted, kind));
    };

    if !hello.pairing {
        // A known device reconnecting on its own keys while someone else pairs still works, and
        // the open window does not change what it is trusted for.
        let Some(known) = known else {
            note_failed_attempt(dir);
            bail!("the other device has not opened pairing");
        };
        if !known.enabled {
            bail!("this paired device is disabled on this computer");
        }
        return Ok((Authorization::Trusted, kind));
    }
    if via != "lan" && via != "relay-pair" {
        note_failed_attempt(dir);
        bail!("pairing must use LAN discovery or the temporary pairing-code rendezvous");
    }
    // Whichever side dialled is the initiator, on this end of the check as well.
    let expected = if initiator {
        pairing_proof(&window.code, local_id, remote_id)
    } else {
        pairing_proof(&window.code, remote_id, local_id)
    };
    // Whoever joins a pairing code already knows it, and a desktop may only enter the PC mesh by
    // proving it. An empty proof is only possible for a legacy phone scanning this PC on the LAN,
    // and never while a phone is already trusted or this very device is a second one beside it.
    let proof_required = via == "relay-pair"
        || announced == PeerKind::Desktop
        || !hello.pairing_proof.is_empty()
        || known.as_ref().map_or_else(
            || peers.iter().any(|peer| peer.kind == PeerKind::Phone),
            |peer| peer.kind == PeerKind::Desktop,
        );
    if proof_required && !constant_time_eq(&hello.pairing_proof, &expected) {
        note_failed_attempt(dir);
        bail!("pairing proof does not match the pairing code");
    }
    if known.is_none() && peers.len() >= MAX_PEERS {
        bail!("this computer already trusts the maximum of {MAX_PEERS} devices");
    }
    // A disabled device that proves the code is an explicit re-pair, which re-enables it.
    Ok((
        if known.as_ref().is_some_and(|peer| peer.enabled) {
            Authorization::Trusted
        } else {
            Authorization::Paired
        },
        kind,
    ))
}

/// Records a candidate that passed [`authorize_candidate`]. Only the main loop calls this, so
/// trust changes stay serialized with admission and the pairing window.
pub fn confirm(
    dir: &Path,
    remote_id: &str,
    kind: PeerKind,
    authorization: Authorization,
    name: &str,
) -> Result<()> {
    let _guard = lock_registry();
    let mut peers = peers_locked(dir);
    if authorization != Authorization::Trusted {
        let name = clean_name(name);
        let mut dismissed = dismissed_peers(dir);
        if dismissed.iter().any(|id| id == remote_id) {
            dismissed.retain(|id| id != remote_id);
            write_dismissed(dir, &dismissed)?;
        }
        match peers.iter_mut().find(|peer| peer.id == remote_id) {
            Some(peer) => {
                // An explicit re-pair keeps the kind this computer already trusted and restores a
                // device the user had disabled; a window never upgrades a desktop to a phone.
                peer.enabled = true;
                if !name.is_empty() {
                    peer.name = name;
                }
            }
            None => {
                if peers.len() >= MAX_PEERS {
                    bail!("this computer already trusts the maximum of {MAX_PEERS} devices");
                }
                peers.push(PairedPeer {
                    id: remote_id.to_owned(),
                    name,
                    kind,
                    enabled: true,
                    source: PeerSource::Direct,
                });
            }
        }
        write_peers_locked(dir, &peers)?;
        choose_peer_locked(dir, remote_id)?;
    } else if !name.is_empty() {
        if let Some(peer) = peers.iter_mut().find(|peer| peer.id == remote_id) {
            let name = clean_name(name);
            if peer.name != name {
                peer.name = name;
                write_peers_locked(dir, &peers)?;
            }
        }
    }
    if authorization != Authorization::Trusted && window_with_attempts_locked(dir).is_some() {
        // The window has done its job. A trusted peer reconnecting in the meantime must not use
        // it up, because another device may still be pairing.
        let _ = remove_if_exists(&dir.join(PAIRING_FILE));
    }
    mark_explicit_pairing_model(dir)
}

/// Authoritative peer registry, migrated once from the single-peer files.
pub fn peers(dir: &Path) -> Vec<PairedPeer> {
    let _guard = lock_registry();
    peers_locked(dir)
}

fn peers_locked(dir: &Path) -> Vec<PairedPeer> {
    if let Ok(body) = std::fs::read_to_string(dir.join(PEERS_FILE)) {
        return parse_peers(&body);
    }
    match migrate_legacy_locked(dir) {
        Ok(peers) => peers,
        Err(error) => {
            tracing::warn!(%error, "could not import the legacy paired peer");
            Vec::new()
        }
    }
}

fn migrate_legacy_locked(dir: &Path) -> Result<Vec<PairedPeer>> {
    if dir.join(PEERS_MODEL_FILE).exists() {
        return Ok(Vec::new());
    }
    let Some(id) = read_one_line(&dir.join(PEER_FILE)).filter(|value| valid_id(value)) else {
        return Ok(Vec::new());
    };
    let peer = PairedPeer {
        id: id.clone(),
        name: read_one_line(&dir.join(PEER_NAME_FILE)).unwrap_or_default(),
        kind: PeerKind::Phone,
        enabled: true,
        source: PeerSource::Direct,
    };
    write_peers_locked(dir, std::slice::from_ref(&peer))?;
    if selected_peer(dir).is_none() {
        choose_peer_locked(dir, &id)?;
    }
    Ok(vec![peer])
}

fn parse_peers(body: &str) -> Vec<PairedPeer> {
    let mut peers: Vec<PairedPeer> = Vec::new();
    for line in body.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 5 {
            continue;
        }
        let id = fields[0];
        let Some(kind) = PeerKind::parse(fields[2]) else {
            continue;
        };
        let Some(source) = PeerSource::parse(fields[4]) else {
            continue;
        };
        let enabled = match fields[3] {
            "1" => true,
            "0" => false,
            _ => continue,
        };
        let Ok(raw_name) = URL_SAFE_NO_PAD.decode(fields[1]) else {
            continue;
        };
        let Ok(name) = String::from_utf8(raw_name) else {
            continue;
        };
        if !valid_id(id) || peers.iter().any(|peer| peer.id == id) || peers.len() == MAX_PEERS {
            continue;
        }
        peers.push(PairedPeer {
            id: id.to_owned(),
            name: clean_name(&name),
            kind,
            enabled,
            source,
        });
    }
    peers
}

fn encode_peers(peers: &[PairedPeer]) -> String {
    peers
        .iter()
        .map(|peer| {
            format!(
                "{}\t{}\t{}\t{}\t{}\n",
                peer.id,
                URL_SAFE_NO_PAD.encode(peer.name.as_bytes()),
                peer.kind.as_str(),
                if peer.enabled { 1 } else { 0 },
                peer.source.as_str(),
            )
        })
        .collect()
}

fn write_peers(dir: &Path, peers: &[PairedPeer]) -> Result<()> {
    let _guard = lock_registry();
    write_peers_locked(dir, peers)
}

fn write_peers_locked(dir: &Path, peers: &[PairedPeer]) -> Result<()> {
    atomic_write(&dir.join(PEERS_FILE), &encode_peers(peers))?;
    if !dir.join(PEERS_MODEL_FILE).exists() {
        atomic_write(&dir.join(PEERS_MODEL_FILE), "1\n")?;
    }
    // Compatibility derivation: old tooling reads exactly one phone from these two files.
    let phone = peers
        .iter()
        .find(|peer| {
            peer.kind == PeerKind::Phone && Some(peer.id.as_str()) == selected_peer(dir).as_deref()
        })
        .or_else(|| peers.iter().find(|peer| peer.kind == PeerKind::Phone));
    match phone {
        Some(phone) => {
            atomic_write(&dir.join(PEER_FILE), &(phone.id.clone() + "\n"))?;
            if phone.name.is_empty() {
                remove_if_exists(&dir.join(PEER_NAME_FILE))?;
            } else {
                atomic_write(&dir.join(PEER_NAME_FILE), &(phone.name.clone() + "\n"))?;
            }
        }
        None => {
            remove_if_exists(&dir.join(PEER_FILE))?;
            remove_if_exists(&dir.join(PEER_NAME_FILE))?;
        }
    }
    Ok(())
}

pub fn selected_peer(dir: &Path) -> Option<String> {
    read_one_line(&dir.join(SELECTED_FILE)).filter(|value| valid_id(value))
}

pub fn choose_peer(dir: &Path, id: &str) -> Result<()> {
    let _guard = lock_registry();
    choose_peer_locked(dir, id)
}

fn choose_peer_locked(dir: &Path, id: &str) -> Result<()> {
    if !peers_locked(dir).iter().any(|peer| peer.id == id) {
        bail!("no such paired device");
    }
    atomic_write(&dir.join(SELECTED_FILE), &(id.to_owned() + "\n"))
}

pub fn set_peer_enabled(dir: &Path, id: &str, enabled: bool) -> Result<bool> {
    let _guard = lock_registry();
    let mut peers = peers_locked(dir);
    let Some(peer) = peers.iter_mut().find(|peer| peer.id == id) else {
        bail!("no such paired device");
    };
    if peer.enabled == enabled {
        return Ok(false);
    }
    peer.enabled = enabled;
    write_peers_locked(dir, &peers)?;
    Ok(true)
}

pub fn enable_all_peers(dir: &Path) -> Result<usize> {
    let _guard = lock_registry();
    let mut peers = peers_locked(dir);
    let mut changed = 0;
    for peer in &mut peers {
        if !peer.enabled {
            peer.enabled = true;
            changed += 1;
        }
    }
    if changed > 0 {
        write_peers_locked(dir, &peers)?;
    }
    Ok(changed)
}

/// Locally forgets one device. A desktop that this PC learned from the phone's roster is also
/// tombstoned, so the next roster cannot silently add it back. The tombstone is durable before the
/// registry loses the peer: a crash in between must not let the roster resurrect the device.
pub fn forget_peer(dir: &Path, id: &str) -> Result<bool> {
    let _guard = lock_registry();
    let peers = peers_locked(dir);
    let Some(peer) = peers.iter().find(|peer| peer.id == id).cloned() else {
        return Ok(false);
    };
    if peer.kind == PeerKind::Desktop && peer.source == PeerSource::Phone {
        let mut dismissed = dismissed_peers(dir);
        dismissed.retain(|value| value != id);
        dismissed.push(id.to_owned());
        // A full ring drops the oldest tombstone, never the id the user just forgot here.
        let excess = dismissed.len().saturating_sub(MAX_DISMISSED);
        dismissed.drain(..excess);
        write_dismissed(dir, &dismissed)?;
    }
    let remaining: Vec<PairedPeer> = peers.into_iter().filter(|peer| peer.id != id).collect();
    write_peers_locked(dir, &remaining)?;
    if selected_peer(dir).as_deref() == Some(id) {
        match remaining.first() {
            Some(next) => choose_peer_locked(dir, &next.id)?,
            None => {
                remove_if_exists(&dir.join(SELECTED_FILE))?;
            }
        }
    }
    Ok(true)
}

fn dismissed_peers(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join(DISMISSED_FILE))
        .map(|body| {
            body.lines()
                .map(str::trim)
                .filter(|value| valid_id(value))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn write_dismissed(dir: &Path, dismissed: &[String]) -> Result<()> {
    if dismissed.is_empty() {
        remove_if_exists(&dir.join(DISMISSED_FILE))?;
        return Ok(());
    }
    let body: String = dismissed.iter().map(|id| id.clone() + "\n").collect();
    atomic_write(&dir.join(DISMISSED_FILE), &body)
}

/// Applies the roster the paired phone pushes whenever its own paired-device list changes.
///
/// Only the persisted phone may do this; a desktop session has no authority over this PC's trust.
/// Desktop entries the phone dropped are removed (and their local tombstone cleared), valid new
/// desktops are added disabled-preserving, and locally forgotten ids stay forgotten.
pub fn apply_phone_roster(
    dir: &Path,
    local_id: &str,
    caller_id: &str,
    roster: &pb::PeerRoster,
) -> Result<Vec<String>> {
    let _guard = lock_registry();
    let peers = peers_locked(dir);
    if !peers
        .iter()
        .any(|peer| peer.id == caller_id && peer.kind == PeerKind::Phone)
    {
        bail!("only the paired phone may update this computer's peer roster");
    }
    if !valid_id(local_id) {
        bail!("local device id is invalid");
    }
    let listed: Vec<&str> = roster
        .peers
        .iter()
        .map(|entry| entry.device_id.as_str())
        .collect();
    let removed: Vec<String> = peers
        .iter()
        .filter(|peer| {
            peer.kind == PeerKind::Desktop
                && peer.source == PeerSource::Phone
                && !listed.contains(&peer.id.as_str())
        })
        .map(|peer| peer.id.clone())
        .collect();
    let dismissed = dismissed_peers(dir);
    let mut next: Vec<PairedPeer> = peers
        .clone()
        .into_iter()
        .filter(|peer| !removed.iter().any(|id| id == &peer.id))
        .collect();
    for entry in &roster.peers {
        if entry.device_id == local_id
            || !valid_id(&entry.device_id)
            || PeerKind::announced(entry.device_kind) != PeerKind::Desktop
            || dismissed.iter().any(|id| id == &entry.device_id)
        {
            continue;
        }
        let name = clean_name(&entry.device_name);
        if let Some(existing) = next.iter().position(|peer| peer.id == entry.device_id) {
            // An existing peer keeps its enabled flag and how this PC learned about it.
            if !name.is_empty() && next[existing].name != name {
                next[existing].name = name;
            }
        } else if next.len() < MAX_PEERS {
            next.push(PairedPeer {
                id: entry.device_id.clone(),
                name,
                kind: PeerKind::Desktop,
                enabled: true,
                source: PeerSource::Phone,
            });
        }
    }
    if next != peers {
        write_peers_locked(dir, &next)?;
    }
    // A tombstone only shields this PC from the phone re-adding a device the user removed here.
    // Once the phone drops that device itself, the tombstone is stale and would block a future
    // re-pair, so only ids the incoming roster still lists survive.
    let kept: Vec<String> = dismissed
        .iter()
        .filter(|id| listed.contains(&id.as_str()))
        .cloned()
        .collect();
    if kept != dismissed {
        write_dismissed(dir, &kept)?;
    }
    Ok(removed)
}

pub fn pairing_rendezvous(code: &str) -> String {
    let normalized = normalize_code(code);
    let mut hash = Sha256::new();
    hash.update(PAIRING_DOMAIN);
    hash.update(normalized.as_bytes());
    URL_SAFE_NO_PAD.encode(hash.finalize())
}

/// Binds a pairing code to the two Noise identities it is pairing. The initiator is the device
/// that dialled; both sides compute the same digest, and the relay splicing them can produce
/// neither without the code.
pub fn pairing_proof(code: &str, initiator_id: &str, responder_id: &str) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(
        format!(
            "{PROOF_DOMAIN}:{}:{initiator_id}:{responder_id}",
            normalize_code(code)
        )
        .as_bytes(),
    );
    hash.finalize().to_vec()
}

pub(crate) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

pub fn normalize_code(code: &str) -> String {
    code.chars()
        .filter(|value| value.is_ascii_digit())
        .collect()
}

pub fn format_code(code: &str) -> String {
    normalize_code(code)
}

fn clean_name(name: &str) -> String {
    name.replace(['\t', '\r', '\n'], " ")
        .trim()
        .chars()
        .take(64)
        .collect()
}

fn generate_code() -> Result<String> {
    // snow already owns the CSPRNG used for Conduit's Noise identities. Reuse that dependency to
    // obtain fresh entropy instead of adding a resident/runtime RNG crate for a two-minute code.
    // Six decimal digits are deliberately short for phone entry; the Relay only reveals a hit by
    // successfully splicing the exact rendezvous, and the authenticated Noise hello still has to
    // prove that both endpoints explicitly entered pairing before the peer is persisted.
    let params = "Noise_XX_25519_ChaChaPoly_BLAKE2s".parse()?;
    let random = snow::Builder::new(params).generate_keypair()?;
    let digest = Sha256::digest(&random.public);
    let value =
        u32::from_be_bytes(digest[..4].try_into().expect("SHA-256 is 32 bytes")) % 1_000_000;
    Ok(format!("{value:06}"))
}

fn pairing_window(dir: &Path) -> Option<PairingWindow> {
    let _guard = lock_registry();
    window_with_attempts_locked(dir).map(|(window, _)| window)
}

fn window_with_attempts(dir: &Path) -> Option<(PairingWindow, u32)> {
    let _guard = lock_registry();
    window_with_attempts_locked(dir)
}

fn window_with_attempts_locked(dir: &Path) -> Option<(PairingWindow, u32)> {
    let path = dir.join(PAIRING_FILE);
    let text = std::fs::read_to_string(&path).ok()?;
    let mut expires_ms = None;
    let mut code = None;
    let mut attempts = 0;
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            // Compatibility with the first development implementation, which stored only the
            // expiry as a bare integer. It cannot support code pairing, so treat it as expired.
            let _ = std::fs::remove_file(&path);
            return None;
        };
        match key.trim() {
            "expires_ms" => expires_ms = value.trim().parse::<u128>().ok(),
            "code" => {
                let raw = value.trim();
                if raw.len() != CODE_LEN || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
                    let _ = std::fs::remove_file(&path);
                    return None;
                }
                code = Some(raw.to_owned());
            }
            "attempts" => attempts = value.trim().parse::<u32>().unwrap_or(0),
            _ => {}
        }
    }
    let expires_ms = expires_ms?;
    let code = code?;
    if expires_ms <= now_ms() {
        let _ = std::fs::remove_file(path);
        return None;
    }
    Some((
        PairingWindow {
            rendezvous: pairing_rendezvous(&code),
            code,
            expires_ms,
        },
        attempts,
    ))
}

/// One rejected pairing check. Five of them close the window, so an online guesser cannot keep a
/// six-digit code open indefinitely.
fn note_failed_attempt(dir: &Path) {
    let _guard = lock_registry();
    let Some((window, attempts)) = window_with_attempts_locked(dir) else {
        return;
    };
    if attempts + 1 >= MAX_ATTEMPTS {
        let _ = remove_if_exists(&dir.join(PAIRING_FILE));
        tracing::warn!(
            attempts = attempts + 1,
            "pairing window closed after repeated failures"
        );
        return;
    }
    let body = format!(
        "expires_ms={}\ncode={}\nattempts={}\n",
        window.expires_ms,
        window.code,
        attempts + 1
    );
    if let Err(error) = atomic_write(&dir.join(PAIRING_FILE), &body) {
        tracing::warn!(%error, "could not record a failed pairing attempt");
    }
}

fn mark_explicit_pairing_model(dir: &Path) -> Result<()> {
    atomic_write(&dir.join(PAIRING_MODEL_FILE), "1\n")
}

fn valid_id(value: &str) -> bool {
    value.len() == ID_LEN
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn read_one_line(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
}

/// Crash-safe replacement: the temporary file is durable before the rename swaps it in, and the
/// target is never removed first, so a crash leaves either the old or the new file.
fn atomic_write(path: &Path, body: &str) -> Result<()> {
    let parent = path.parent().context("pairing file has no parent")?;
    std::fs::create_dir_all(parent)?;
    let tmp = temp_path(path);
    {
        let mut file =
            std::fs::File::create(&tmp).with_context(|| format!("writing {}", tmp.display()))?;
        file.write_all(body.as_bytes())
            .with_context(|| format!("writing {}", tmp.display()))?;
        file.sync_all()
            .with_context(|| format!("flushing {}", tmp.display()))?;
    }
    std::fs::rename(&tmp, path)
        .with_context(|| format!("moving {} to {}", tmp.display(), path.display()))
}

fn temp_path(path: &Path) -> PathBuf {
    // Unique per call: two sessions may write the same target at once, and a shared temporary name
    // would let one rename another's half-written file into place.
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(
        ".{}-{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    PathBuf::from(name)
}

fn remove_if_exists(path: &Path) -> Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("removing {}", path.display())),
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

/// Explicit, offline-only repair for a second PC created by copying an existing
/// Conduit data directory. Never invoked automatically; only the copied
/// installation should be re-keyed. Identity and trust files are backed up beside the data
/// directory before the new Noise static key is generated. Transfers, clipboard history and
/// settings are untouched.
pub struct CloneRepair {
    pub backup_dir: PathBuf,
    pub new_device_id: String,
    pub new_fingerprint: String,
}

/// Every file that describes *which* devices this PC trusts. Backed up together, cleared together.
const TRUST_FILES: [&str; 8] = [
    PEER_FILE,
    PEER_NAME_FILE,
    PEERS_FILE,
    SELECTED_FILE,
    DISMISSED_FILE,
    STATUS_PEERS_FILE,
    PAIRING_FILE,
    PAIRING_MODEL_FILE,
];

pub fn repair_cloned_identity(dir: &Path) -> Result<CloneRepair> {
    use crate::wire;
    let identity = dir.join("identity.bin");
    let old = std::fs::read(&identity)
        .with_context(|| format!("reading existing identity {}", identity.display()))?;
    if old.len() != 64 {
        bail!("existing Noise identity is not 64 bytes; refusing replacement");
    }
    let original_id = wire::device_id(&old[32..]);
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let backup = dir.join(format!(
        "pre-clone-repair-backup-{}-{}",
        std::process::id(),
        unique
    ));
    std::fs::create_dir(&backup)
        .with_context(|| format!("creating identity backup {}", backup.display()))?;
    for file in TRUST_FILES {
        let source = dir.join(file);
        if source.is_file() {
            std::fs::copy(&source, backup.join(file))
                .with_context(|| format!("backing up {}", file))?;
        }
    }
    std::fs::rename(&identity, backup.join("identity.bin"))
        .with_context(|| "moving imported Noise identity to local backup")?;
    let result = (|| -> Result<(String, String)> {
        let fresh = wire::load_or_create_identity(&identity)?;
        let new_id = wire::device_id(&fresh.public);
        if new_id == original_id {
            bail!("identity regeneration unexpectedly produced the same public key");
        }
        forget(dir)?;
        Ok((new_id, wire::fingerprint(&fresh.public)))
    })();
    match result {
        Ok((new_device_id, new_fingerprint)) => Ok(CloneRepair {
            backup_dir: backup,
            new_device_id,
            new_fingerprint,
        }),
        Err(error) => {
            let _ = std::fs::remove_file(&identity);
            std::fs::rename(backup.join("identity.bin"), &identity)
                .with_context(|| "restoring original Noise identity after failed repair")?;
            for file in TRUST_FILES {
                let original = backup.join(file);
                if original.is_file() {
                    std::fs::copy(original, dir.join(file))?;
                }
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "conduit-pairing-{name}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn hello(kind: PeerKind, pairing: bool, proof: Vec<u8>) -> pb::PairRequest {
        pb::PairRequest {
            pairing,
            device_kind: match kind {
                PeerKind::Desktop => pb::DeviceKind::Desktop as i32,
                PeerKind::Phone => pb::DeviceKind::Phone as i32,
            },
            pairing_proof: proof,
            ..Default::default()
        }
    }

    fn ids() -> (String, String, String) {
        ("A".repeat(ID_LEN), "B".repeat(ID_LEN), "C".repeat(ID_LEN))
    }

    #[test]
    fn pairing_code_normalizes_and_has_a_stable_rendezvous() {
        assert_eq!(normalize_code("12 34-56"), "123456");
        assert_eq!(pairing_rendezvous("123-456"), pairing_rendezvous("123456"));
        assert_eq!(
            pairing_rendezvous("123456"),
            "3sLbGZON6YWYSIrLdCIGl7TWmbRLGLVRBqCwooefYBY"
        );
        assert_eq!(pairing_rendezvous("123456").len(), ID_LEN);
    }

    #[test]
    fn pairing_proof_binds_both_ids_and_the_normalized_code() {
        let (a, b, c) = ids();
        let proof = pairing_proof("12 34-56", &a, &b);
        assert_eq!(proof.len(), 32);
        assert_eq!(proof, pairing_proof("123456", &a, &b));
        assert_ne!(proof, pairing_proof("123456", &b, &a));
        assert_ne!(proof, pairing_proof("123456", &a, &c));
        let wrong = pairing_proof("654321", &a, &b);
        assert!(!constant_time_eq(&proof, &wrong));
        assert!(!constant_time_eq(&proof, &proof[..31]));
        assert!(constant_time_eq(&proof, &proof.clone()));
    }

    #[test]
    fn desktop_window_pairing_requires_a_matching_proof() {
        let dir = temp("proof");
        let (a, b, _) = ids();
        let window = start(&dir).unwrap();
        let candidate = hello(PeerKind::Desktop, true, pairing_proof(&window.code, &b, &a));
        let (authorization, kind) =
            authorize_candidate(&dir, &a, &b, "relay-pair", false, &candidate).unwrap();
        assert_eq!(authorization, Authorization::Paired);
        assert_eq!(kind, PeerKind::Desktop);
        confirm(&dir, &b, kind, authorization, "Studio PC").unwrap();
        assert_eq!(peers(&dir).len(), 1);
        assert_eq!(selected_peer(&dir).as_deref(), Some(b.as_str()));
        assert!(!state(&dir).pairing);

        // Reconnecting as a trusted peer no longer needs the window or a proof.
        let (authorization, kind) = authorize_candidate(
            &dir,
            &a,
            &b,
            "relay",
            false,
            &hello(PeerKind::Desktop, false, vec![]),
        )
        .unwrap();
        assert_eq!(authorization, Authorization::Trusted);
        assert_eq!(kind, PeerKind::Desktop);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn five_wrong_proofs_close_the_pairing_window() {
        let dir = temp("attempts");
        let (a, b, c) = ids();
        start(&dir).unwrap();
        for _ in 0..4 {
            let candidate = hello(PeerKind::Desktop, true, pairing_proof("000000", &b, &a));
            assert!(authorize_candidate(&dir, &a, &b, "lan", false, &candidate).is_err());
            assert!(state(&dir).pairing);
        }
        let candidate = hello(PeerKind::Desktop, true, pairing_proof("000000", &c, &a));
        assert!(authorize_candidate(&dir, &a, &c, "lan", false, &candidate).is_err());
        assert!(!state(&dir).pairing, "the window must have closed itself");
        assert!(!dir.join(PAIRING_FILE).exists());
        // A blocked peer is never persisted by a failed check.
        assert!(peers(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rejected_lan_candidate_spends_a_pairing_attempt() {
        let dir = temp("lan-attempts");
        let (a, b, _) = ids();
        start(&dir).unwrap();
        // A candidate that never opened pairing is a failed attempt, exactly like a wrong proof,
        // so a LAN socket cannot be used to keep a window open while guessing.
        for _ in 0..4 {
            assert!(authorize_candidate(
                &dir,
                &a,
                &b,
                "lan",
                false,
                &hello(PeerKind::Phone, false, vec![])
            )
            .is_err());
            assert!(state(&dir).pairing);
        }
        assert!(authorize_candidate(
            &dir,
            &a,
            &b,
            "lan",
            false,
            &hello(PeerKind::Phone, false, vec![])
        )
        .is_err());
        assert!(!state(&dir).pairing, "the window must have closed itself");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_phone_must_prove_the_code_on_the_rendezvous_but_not_on_the_lan() {
        let dir = temp("phone-window");
        let (a, b, c) = ids();
        start(&dir).unwrap();
        // The temporary rendezvous is derived from the code, so whoever joins it knows the code.
        assert!(authorize_candidate(
            &dir,
            &a,
            &b,
            "relay-pair",
            false,
            &hello(PeerKind::Phone, true, vec![])
        )
        .is_err());
        // The long-lived relay slot is never a pairing transport.
        assert!(authorize_candidate(
            &dir,
            &a,
            &b,
            "relay",
            false,
            &hello(PeerKind::Phone, true, vec![])
        )
        .is_err());
        // The first phone may still pair by scanning this PC on the LAN, which reveals no code.
        let (authorization, kind) = authorize_candidate(
            &dir,
            &a,
            &b,
            "lan",
            false,
            &hello(PeerKind::Phone, true, vec![]),
        )
        .unwrap();
        assert_eq!(
            (authorization, kind),
            (Authorization::Paired, PeerKind::Phone)
        );
        confirm(&dir, &b, kind, authorization, "Phone").unwrap();
        assert_eq!(peer_id(&dir).as_deref(), Some(b.as_str()));
        assert_eq!(peer_name(&dir).as_deref(), Some("Phone"));
        // With the phone trusted, an empty-proof LAN candidate must not become a second phone.
        let window = start(&dir).unwrap();
        assert!(authorize_candidate(
            &dir,
            &a,
            &c,
            "lan",
            false,
            &hello(PeerKind::Phone, true, vec![])
        )
        .is_err());
        // A phone that does prove the code still pairs over the LAN.
        let proof = pairing_proof(&window.code, &c, &a);
        let (authorization, kind) = authorize_candidate(
            &dir,
            &a,
            &c,
            "lan",
            false,
            &hello(PeerKind::Phone, true, proof),
        )
        .unwrap();
        assert_eq!(
            (authorization, kind),
            (Authorization::Paired, PeerKind::Phone)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ordinary_relay_cannot_replace_a_paired_peer() {
        let dir = temp("ordinary-relay");
        let (a, b, c) = ids();
        start(&dir).unwrap();
        let (authorization, kind) = authorize_candidate(
            &dir,
            &a,
            &b,
            "lan",
            false,
            &hello(PeerKind::Phone, true, vec![]),
        )
        .unwrap();
        confirm(&dir, &b, kind, authorization, "Phone").unwrap();
        let (authorization, kind) = authorize_candidate(
            &dir,
            &a,
            &b,
            "lan",
            false,
            &hello(PeerKind::Phone, true, vec![]),
        )
        .unwrap();
        assert_eq!(
            (authorization, kind),
            (Authorization::Trusted, PeerKind::Phone)
        );
        assert!(authorize_candidate(
            &dir,
            &a,
            &b,
            "lan",
            false,
            &hello(PeerKind::Phone, false, vec![])
        )
        .is_ok());
        // A different device is still unknown, with or without a stale pairing flag.
        assert!(authorize_candidate(
            &dir,
            &a,
            &c,
            "lan",
            false,
            &hello(PeerKind::Phone, false, vec![])
        )
        .is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_paired_desktop_keeps_its_kind_while_a_pairing_window_is_open() {
        let dir = temp("kind-lock");
        let (a, b, _) = ids();
        let window = start(&dir).unwrap();
        let proof = pairing_proof(&window.code, &b, &a);
        let (authorization, kind) = authorize_candidate(
            &dir,
            &a,
            &b,
            "relay-pair",
            false,
            &hello(PeerKind::Desktop, true, proof),
        )
        .unwrap();
        confirm(&dir, &b, kind, authorization, "Studio").unwrap();
        // The same device re-announcing itself as a phone must stay the desktop this PC trusted.
        let window = start(&dir).unwrap();
        let proof = pairing_proof(&window.code, &b, &a);
        let (authorization, kind) = authorize_candidate(
            &dir,
            &a,
            &b,
            "relay-pair",
            false,
            &hello(PeerKind::Phone, true, proof),
        )
        .unwrap();
        assert_eq!(
            (authorization, kind),
            (Authorization::Trusted, PeerKind::Desktop)
        );
        confirm(&dir, &b, kind, authorization, "Studio").unwrap();
        assert_eq!(peers(&dir)[0].kind, PeerKind::Desktop);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_disabled_desktop_re_enables_only_by_proving_the_code() {
        let dir = temp("repair-disable");
        let (a, b, _) = ids();
        let window = start(&dir).unwrap();
        let proof = pairing_proof(&window.code, &b, &a);
        let (authorization, kind) = authorize_candidate(
            &dir,
            &a,
            &b,
            "relay-pair",
            false,
            &hello(PeerKind::Desktop, true, proof),
        )
        .unwrap();
        confirm(&dir, &b, kind, authorization, "Studio").unwrap();
        assert!(set_peer_enabled(&dir, &b, false).unwrap());
        // Outside a pairing the disabled device is refused on its own keys.
        assert!(authorize_candidate(
            &dir,
            &a,
            &b,
            "relay",
            false,
            &hello(PeerKind::Desktop, false, vec![])
        )
        .is_err());
        // An open window alone is not enough: a desktop must prove the code to come back.
        let window = start(&dir).unwrap();
        assert!(authorize_candidate(
            &dir,
            &a,
            &b,
            "lan",
            false,
            &hello(PeerKind::Desktop, true, vec![])
        )
        .is_err());
        let proof = pairing_proof(&window.code, &b, &a);
        let (authorization, kind) = authorize_candidate(
            &dir,
            &a,
            &b,
            "lan",
            false,
            &hello(PeerKind::Desktop, true, proof),
        )
        .unwrap();
        assert_eq!(
            (authorization, kind),
            (Authorization::Paired, PeerKind::Desktop)
        );
        confirm(&dir, &b, kind, authorization, "Studio").unwrap();
        assert!(peers(&dir).iter().all(|peer| peer.enabled));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unpaired_lan_is_rejected_but_legacy_relay_migrates_once() {
        let dir = temp("legacy");
        let (a, b, c) = ids();
        assert!(authorize_candidate(
            &dir,
            &a,
            &b,
            "lan",
            false,
            &hello(PeerKind::Phone, false, vec![])
        )
        .is_err());
        let candidate = hello(PeerKind::Phone, false, vec![]);
        let (authorization, kind) =
            authorize_candidate(&dir, &a, &b, "relay", false, &candidate).unwrap();
        assert_eq!(authorization, Authorization::LegacyRelayMigration);
        assert!(peers(&dir).is_empty());
        confirm(&dir, &b, kind, authorization, "Phone").unwrap();
        assert_eq!(peer_id(&dir).as_deref(), Some(b.as_str()));
        assert!(authorize_candidate(&dir, &a, &b, "relay", false, &candidate).is_ok());
        assert!(authorize_candidate(&dir, &a, &c, "relay", false, &candidate).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_single_peer_files_migrate_once_as_the_phone() {
        let dir = temp("migration");
        let (_, b, _) = ids();
        std::fs::write(dir.join(PEER_FILE), &b).unwrap();
        std::fs::write(dir.join(PEER_NAME_FILE), "My phone\n").unwrap();
        let migrated = peers(&dir);
        assert_eq!(migrated.len(), 1);
        assert_eq!(migrated[0].id, b);
        assert_eq!(migrated[0].name, "My phone");
        assert_eq!(migrated[0].kind, PeerKind::Phone);
        assert_eq!(migrated[0].source, PeerSource::Direct);
        assert!(migrated[0].enabled);
        assert_eq!(selected_peer(&dir).as_deref(), Some(b.as_str()));
        assert!(dir.join(PEERS_MODEL_FILE).exists());
        // Forgetting everything must not resurrect the legacy phone from the import.
        assert!(forget(&dir).unwrap());
        assert!(peers(&dir).is_empty());
        assert!(peer_id(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn registry_round_trips_and_enforces_the_peer_cap() {
        let dir = temp("cap");
        let mut all = Vec::new();
        for index in 0..MAX_PEERS + 2 {
            all.push(PairedPeer {
                id: format!("{index:0width$}", width = ID_LEN),
                name: format!("Peer {index}\twith a tab"),
                kind: if index % 2 == 0 {
                    PeerKind::Phone
                } else {
                    PeerKind::Desktop
                },
                enabled: index % 3 != 0,
                source: PeerSource::Direct,
            });
        }
        write_peers(&dir, &all).unwrap();
        let read_back = peers(&dir);
        assert_eq!(read_back.len(), MAX_PEERS);
        assert_eq!(read_back[1].name, "Peer 1 with a tab");
        assert_eq!(read_back[0].kind, PeerKind::Phone);
        assert!(!read_back[3].enabled);
        assert_eq!(peer_id(&dir), Some(read_back[0].id.clone()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tombstoned_peer_is_not_resurrected_by_a_later_roster() {
        let dir = temp("tombstone");
        let (local, phone, desktop) = ids();
        write_peers(
            &dir,
            &[
                PairedPeer {
                    id: phone.clone(),
                    name: "Phone".into(),
                    kind: PeerKind::Phone,
                    enabled: true,
                    source: PeerSource::Direct,
                },
                PairedPeer {
                    id: desktop.clone(),
                    name: "Studio".into(),
                    kind: PeerKind::Desktop,
                    enabled: true,
                    source: PeerSource::Phone,
                },
            ],
        )
        .unwrap();
        assert!(forget_peer(&dir, &desktop).unwrap());
        assert!(dismissed_peers(&dir).contains(&desktop));
        assert!(!forget_peer(&dir, &desktop).unwrap(), "already forgotten");
        let roster = pb::PeerRoster {
            peers: vec![pb::PeerInfo {
                device_id: desktop.clone(),
                device_name: "Studio".into(),
                device_kind: pb::DeviceKind::Desktop as i32,
                ..Default::default()
            }],
        };
        let removed = apply_phone_roster(&dir, &local, &phone, &roster).unwrap();
        assert!(removed.is_empty());
        assert!(!peers(&dir).iter().any(|peer| peer.id == desktop));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_full_tombstone_ring_keeps_the_device_the_user_just_forgot() {
        let dir = temp("tombstone-ring");
        let (_, phone, desktop) = ids();
        let stale: Vec<String> = (0..MAX_DISMISSED)
            .map(|index| format!("{index:0width$}", width = ID_LEN))
            .collect();
        write_dismissed(&dir, &stale).unwrap();
        write_peers(
            &dir,
            &[
                PairedPeer {
                    id: phone.clone(),
                    name: "Phone".into(),
                    kind: PeerKind::Phone,
                    enabled: true,
                    source: PeerSource::Direct,
                },
                PairedPeer {
                    id: desktop.clone(),
                    name: "Studio".into(),
                    kind: PeerKind::Desktop,
                    enabled: true,
                    source: PeerSource::Phone,
                },
            ],
        )
        .unwrap();
        assert!(forget_peer(&dir, &desktop).unwrap());
        let dismissed = dismissed_peers(&dir);
        assert_eq!(dismissed.len(), MAX_DISMISSED);
        assert!(dismissed.contains(&desktop));
        assert!(
            !dismissed.contains(&stale[0]),
            "the oldest tombstone is the one that goes"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_roster_that_no_longer_lists_a_desktop_clears_its_tombstone() {
        let dir = temp("stale-tombstone");
        let (local, phone, desktop) = ids();
        write_peers(
            &dir,
            &[PairedPeer {
                id: phone.clone(),
                name: "Phone".into(),
                kind: PeerKind::Phone,
                enabled: true,
                source: PeerSource::Direct,
            }],
        )
        .unwrap();
        write_dismissed(&dir, &[desktop.clone()]).unwrap();
        let listing = pb::PeerRoster {
            peers: vec![pb::PeerInfo {
                device_id: desktop.clone(),
                device_name: "Studio".into(),
                device_kind: pb::DeviceKind::Desktop as i32,
                ..Default::default()
            }],
        };
        // The phone still lists it, so the local tombstone keeps it out.
        apply_phone_roster(&dir, &local, &phone, &listing).unwrap();
        assert!(!peers(&dir).iter().any(|peer| peer.id == desktop));
        assert_eq!(dismissed_peers(&dir), vec![desktop.clone()]);
        // Once the phone forgets it too, the tombstone is stale and must not block a re-pair.
        apply_phone_roster(&dir, &local, &phone, &pb::PeerRoster::default()).unwrap();
        assert!(dismissed_peers(&dir).is_empty());
        apply_phone_roster(&dir, &local, &phone, &listing).unwrap();
        assert!(peers(&dir).iter().any(|peer| peer.id == desktop));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn roster_is_only_accepted_from_the_paired_phone() {
        let dir = temp("roster-authority");
        let (local, phone, desktop) = ids();
        write_peers(
            &dir,
            &[
                PairedPeer {
                    id: phone.clone(),
                    name: "Phone".into(),
                    kind: PeerKind::Phone,
                    enabled: true,
                    source: PeerSource::Direct,
                },
                PairedPeer {
                    id: desktop.clone(),
                    name: "Studio".into(),
                    kind: PeerKind::Desktop,
                    enabled: true,
                    source: PeerSource::Direct,
                },
            ],
        )
        .unwrap();
        let roster = pb::PeerRoster::default();
        assert!(apply_phone_roster(&dir, &local, &desktop, &roster).is_err());
        assert!(apply_phone_roster(&dir, &local, &"Z".repeat(ID_LEN), &roster).is_err());
        assert!(apply_phone_roster(&dir, &local, &phone, &roster).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn roster_upserts_desktops_removes_dropped_ones_and_keeps_local_preferences() {
        let dir = temp("roster");
        let (local, phone, dropped) = ids();
        let added = "D".repeat(ID_LEN);
        let kept = "E".repeat(ID_LEN);
        write_peers(
            &dir,
            &[
                PairedPeer {
                    id: phone.clone(),
                    name: "Phone".into(),
                    kind: PeerKind::Phone,
                    enabled: true,
                    source: PeerSource::Direct,
                },
                PairedPeer {
                    id: dropped.clone(),
                    name: "Gone".into(),
                    kind: PeerKind::Desktop,
                    enabled: true,
                    source: PeerSource::Phone,
                },
                PairedPeer {
                    id: kept.clone(),
                    name: "Old name".into(),
                    kind: PeerKind::Desktop,
                    enabled: false,
                    source: PeerSource::Direct,
                },
            ],
        )
        .unwrap();
        let roster = pb::PeerRoster {
            peers: vec![
                pb::PeerInfo {
                    device_id: kept.clone(),
                    device_name: "Renamed".into(),
                    device_kind: pb::DeviceKind::Desktop as i32,
                    ..Default::default()
                },
                pb::PeerInfo {
                    device_id: added.clone(),
                    device_name: "New".into(),
                    device_kind: pb::DeviceKind::Desktop as i32,
                    ..Default::default()
                },
                // The phone must not be able to list this computer or itself as a peer.
                pb::PeerInfo {
                    device_id: local.clone(),
                    device_kind: pb::DeviceKind::Desktop as i32,
                    ..Default::default()
                },
                pb::PeerInfo {
                    device_id: phone.clone(),
                    device_name: "Phone".into(),
                    device_kind: pb::DeviceKind::Phone as i32,
                    ..Default::default()
                },
            ],
        };
        let removed = apply_phone_roster(&dir, &local, &phone, &roster).unwrap();
        assert_eq!(removed, vec![dropped.clone()]);
        let peers = peers(&dir);
        assert!(!peers.iter().any(|peer| peer.id == dropped));
        let kept_peer = peers.iter().find(|peer| peer.id == kept).unwrap();
        assert_eq!(kept_peer.name, "Renamed");
        assert!(!kept_peer.enabled, "a local disable must survive a roster");
        assert_eq!(kept_peer.source, PeerSource::Direct);
        let added_peer = peers.iter().find(|peer| peer.id == added).unwrap();
        assert!(added_peer.enabled);
        assert_eq!(added_peer.source, PeerSource::Phone);
        assert_eq!(added_peer.kind, PeerKind::Desktop);
        assert!(!peers.iter().any(|peer| peer.id == local));
        assert_eq!(peers.len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn enabling_and_forgetting_peers_updates_the_derived_phone_files() {
        let dir = temp("controls");
        let (_, phone, desktop) = ids();
        let peers = [
            PairedPeer {
                id: phone.clone(),
                name: "Phone".into(),
                kind: PeerKind::Phone,
                enabled: true,
                source: PeerSource::Direct,
            },
            PairedPeer {
                id: desktop.clone(),
                name: "Studio".into(),
                kind: PeerKind::Desktop,
                enabled: false,
                source: PeerSource::Phone,
            },
        ];
        write_peers(&dir, &peers).unwrap();
        assert!(set_peer_enabled(&dir, &desktop, true).unwrap());
        assert!(!set_peer_enabled(&dir, &desktop, true).unwrap());
        assert!(set_peer_enabled(&dir, &"Q".repeat(ID_LEN), true).is_err());
        assert!(choose_peer(&dir, &desktop).is_ok());
        assert_eq!(selected_peer(&dir).as_deref(), Some(desktop.as_str()));
        assert_eq!(enable_all_peers(&dir).unwrap(), 0);
        assert_eq!(
            std::fs::read_to_string(dir.join(PEER_FILE)).unwrap(),
            format!("{phone}\n")
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(PEER_NAME_FILE)).unwrap(),
            "Phone\n"
        );
        assert!(forget_peer(&dir, &phone).unwrap());
        assert!(peer_id(&dir).is_none());
        assert!(!dir.join(PEER_FILE).exists());
        assert_eq!(selected_peer(&dir).as_deref(), Some(desktop.as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cloned_workstation_identity_rekey_clears_every_trust_file() {
        let dir = temp("clone-repair");
        let original = crate::wire::load_or_create_identity(&dir.join("identity.bin")).unwrap();
        let id = crate::wire::device_id(&original.public);
        let bytes = std::fs::read(dir.join("identity.bin")).unwrap();
        std::fs::write(dir.join(PEER_FILE), "trusted-phone").unwrap();
        std::fs::write(dir.join(PEER_NAME_FILE), "My phone").unwrap();
        std::fs::write(dir.join(PEERS_FILE), "registry").unwrap();
        std::fs::write(dir.join(SELECTED_FILE), "selected").unwrap();
        std::fs::write(dir.join(DISMISSED_FILE), "dismissed").unwrap();
        std::fs::write(dir.join(STATUS_PEERS_FILE), "snapshot").unwrap();
        std::fs::write(
            dir.join("transfers.tsv"),
            "transfer history must not change",
        )
        .unwrap();
        let fixed = repair_cloned_identity(&dir).unwrap();
        assert_ne!(fixed.new_device_id, id);
        assert_eq!(
            std::fs::read(fixed.backup_dir.join("identity.bin")).unwrap(),
            bytes
        );
        assert_eq!(
            std::fs::read_to_string(fixed.backup_dir.join(PEERS_FILE)).unwrap(),
            "registry"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("transfers.tsv")).unwrap(),
            "transfer history must not change"
        );
        for file in [PEER_FILE, PEERS_FILE, SELECTED_FILE, DISMISSED_FILE] {
            assert!(!dir.join(file).exists(), "{file} survived the repair");
        }
        assert!(dir.join(PAIRING_MODEL_FILE).exists());
        let rotated = crate::wire::load_or_create_identity(&dir.join("identity.bin")).unwrap();
        assert_eq!(fixed.new_device_id, crate::wire::device_id(&rotated.public));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
