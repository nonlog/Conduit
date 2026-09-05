//! One-peer trust and explicit pairing state for the Windows companion.
//!
//! Conduit deliberately keeps the current one-phone model: one persisted peer id, one display
//! name, and a short user-opened pairing window. The pairing window can be reached either by LAN
//! discovery or by a temporary Relay rendezvous derived from the human pairing code. No permanent
//! polling is added; the temporary Relay parks exist only for the two-minute window.

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PEER_FILE: &str = "peer.txt";
const PEER_NAME_FILE: &str = "peer-name.txt";
const PAIRING_FILE: &str = "pairing.txt";
/// Marks data directories that have already entered the explicit-pairing model. Without this, an
/// offline phone could resurrect itself through the one-time legacy Relay migration after the user
/// explicitly chose Forget.
const PAIRING_MODEL_FILE: &str = "pairing-v2";
const ID_LEN: usize = 43;
const CODE_LEN: usize = 6;
const PAIRING_DOMAIN: &[u8] = b"conduit-pair-v2:";
pub const WINDOW: Duration = Duration::from_secs(120);

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

pub fn peer_id(dir: &Path) -> Option<String> {
    read_one_line(&dir.join(PEER_FILE)).filter(|value| valid_id(value))
}

pub fn peer_name(dir: &Path) -> Option<String> {
    read_one_line(&dir.join(PEER_NAME_FILE)).filter(|value| !value.is_empty())
}

pub fn start(dir: &Path) -> Result<PairingWindow> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let code = generate_code()?;
    let expires_ms = now_ms().saturating_add(WINDOW.as_millis());
    atomic_write(
        &dir.join(PAIRING_FILE),
        &format!("expires_ms={expires_ms}\ncode={code}\n"),
    )?;
    Ok(PairingWindow {
        rendezvous: pairing_rendezvous(&code),
        code,
        expires_ms,
    })
}

pub fn cancel(dir: &Path) -> Result<bool> {
    remove_if_exists(&dir.join(PAIRING_FILE))
}

pub fn forget(dir: &Path) -> Result<bool> {
    let mut changed = false;
    changed |= remove_if_exists(&dir.join(PEER_FILE))?;
    changed |= remove_if_exists(&dir.join(PEER_NAME_FILE))?;
    changed |= remove_if_exists(&dir.join(PAIRING_FILE))?;
    mark_explicit_pairing_model(dir)?;
    Ok(changed)
}

pub fn remember_name(dir: &Path, name: &str) -> Result<()> {
    let name = name
        .replace(['\r', '\n'], " ")
        .trim()
        .chars()
        .take(64)
        .collect::<String>();
    if name.is_empty() {
        return Ok(());
    }
    atomic_write(&dir.join(PEER_NAME_FILE), &(name + "\n"))
}

/// Decides whether one completed Noise XX handshake may become an application session.
///
/// A different peer is accepted only while the user has explicitly opened pairing on both sides.
/// LAN discovery and the temporary pairing-code Relay rendezvous are both acceptable pairing
/// transports; the normal long-lived Relay rendezvous never is. Existing installations get one
/// compatibility path because old Windows builds had no peer store at all.
pub fn authorize(
    dir: &Path,
    remote_id: &str,
    via: &str,
    remote_pairing: bool,
) -> Result<Authorization> {
    if !valid_id(remote_id) {
        bail!("peer id is not a valid Conduit device id");
    }

    let remembered = peer_id(dir);
    let local_pairing = pairing_window(dir).is_some();
    if local_pairing {
        if via != "lan" && via != "relay-pair" {
            bail!("pairing must use LAN discovery or the temporary pairing-code rendezvous");
        }
        if !remote_pairing {
            bail!("the other device has not opened pairing");
        }
        return Ok(if remembered.as_deref() == Some(remote_id) {
            Authorization::Trusted
        } else {
            Authorization::Paired
        });
    }

    if remembered.as_deref() == Some(remote_id) {
        return Ok(Authorization::Trusted);
    }

    // Upgrade compatibility only: old Windows builds had no peer store at all, while an already
    // paired phone knows this desktop's unguessable rendezvous id and can therefore reach this
    // exact relay parking slot. Pin that phone after the mutual hello succeeds. Fresh installs
    // have a new rendezvous id, so an unrelated phone cannot use this path.
    if remembered.is_none() && via == "relay" && !dir.join(PAIRING_MODEL_FILE).exists() {
        return Ok(Authorization::LegacyRelayMigration);
    }

    bail!("peer is not paired; open pairing on both devices first")
}

pub fn confirm(
    dir: &Path,
    remote_id: &str,
    authorization: Authorization,
    name: &str,
) -> Result<()> {
    if authorization != Authorization::Trusted {
        remember_peer(dir, remote_id)?;
    }
    if pairing_window(dir).is_some() {
        let _ = cancel(dir);
    }
    remember_name(dir, name)?;
    mark_explicit_pairing_model(dir)
}

pub fn pairing_rendezvous(code: &str) -> String {
    let normalized = normalize_code(code);
    let mut hash = Sha256::new();
    hash.update(PAIRING_DOMAIN);
    hash.update(normalized.as_bytes());
    URL_SAFE_NO_PAD.encode(hash.finalize())
}

pub fn normalize_code(code: &str) -> String {
    code.chars().filter(|value| value.is_ascii_digit()).collect()
}

pub fn format_code(code: &str) -> String {
    normalize_code(code)
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
    let value = u32::from_be_bytes(digest[..4].try_into().expect("SHA-256 is 32 bytes")) % 1_000_000;
    Ok(format!("{value:06}"))
}

fn pairing_window(dir: &Path) -> Option<PairingWindow> {
    let path = dir.join(PAIRING_FILE);
    let text = std::fs::read_to_string(&path).ok()?;
    let mut expires_ms = None;
    let mut code = None;
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
            _ => {}
        }
    }
    let expires_ms = expires_ms?;
    let code = code?;
    if code.len() != CODE_LEN {
        let _ = std::fs::remove_file(&path);
        return None;
    }
    if expires_ms <= now_ms() {
        let _ = std::fs::remove_file(path);
        return None;
    }
    Some(PairingWindow {
        rendezvous: pairing_rendezvous(&code),
        code,
        expires_ms,
    })
}

fn mark_explicit_pairing_model(dir: &Path) -> Result<()> {
    atomic_write(&dir.join(PAIRING_MODEL_FILE), "1\n")
}

fn remember_peer(dir: &Path, remote_id: &str) -> Result<()> {
    let previous = peer_id(dir);
    atomic_write(&dir.join(PEER_FILE), &(remote_id.to_owned() + "\n"))?;
    if previous.as_deref() != Some(remote_id) {
        let _ = remove_if_exists(&dir.join(PEER_NAME_FILE));
    }
    Ok(())
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

fn atomic_write(path: &Path, body: &str) -> Result<()> {
    let parent = path.parent().context("pairing file has no parent")?;
    std::fs::create_dir_all(parent)?;
    let tmp = temp_path(path);
    std::fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
    if path.exists() {
        std::fs::remove_file(path).with_context(|| format!("replacing {}", path.display()))?;
    }
    std::fs::rename(&tmp, path)
        .with_context(|| format!("moving {} to {}", tmp.display(), path.display()))
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".tmp");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("conduit-pairing-{name}-{}", std::process::id()))
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
    fn explicit_window_pairs_once_then_pins() {
        let dir = temp("window");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = "A".repeat(ID_LEN);
        let b = "B".repeat(ID_LEN);
        let window = start(&dir).unwrap();
        assert_eq!(window.code.len(), CODE_LEN);
        assert_eq!(window.rendezvous.len(), ID_LEN);
        let authorization = authorize(&dir, &a, "relay-pair", true).unwrap();
        assert_eq!(authorization, Authorization::Paired);
        assert!(state(&dir).pairing);
        confirm(&dir, &a, authorization, "Phone").unwrap();
        assert!(!state(&dir).pairing);
        assert_eq!(
            authorize(&dir, &a, "lan", true).unwrap(),
            Authorization::Trusted
        );
        assert!(authorize(&dir, &b, "lan", false).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ordinary_relay_cannot_replace_a_peer_during_pairing() {
        let dir = temp("ordinary-relay");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        start(&dir).unwrap();
        let id = "E".repeat(ID_LEN);
        assert!(authorize(&dir, &id, "relay", true).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unpaired_lan_is_rejected_but_legacy_relay_migrates() {
        let dir = temp("legacy");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let id = "C".repeat(ID_LEN);
        assert!(authorize(&dir, &id, "lan", false).is_err());
        let authorization = authorize(&dir, &id, "relay", false).unwrap();
        assert_eq!(authorization, Authorization::LegacyRelayMigration);
        assert!(peer_id(&dir).is_none());
        confirm(&dir, &id, authorization, "Phone").unwrap();
        assert_eq!(peer_id(&dir).as_deref(), Some(id.as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn forget_keeps_identity_outside_pair_store() {
        let dir = temp("forget");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let id = "D".repeat(ID_LEN);
        start(&dir).unwrap();
        let authorization = authorize(&dir, &id, "lan", true).unwrap();
        confirm(&dir, &id, authorization, "Phone").unwrap();
        std::fs::write(dir.join("identity.bin"), b"identity").unwrap();
        assert!(forget(&dir).unwrap());
        assert!(peer_id(&dir).is_none());
        assert!(peer_name(&dir).is_none());
        assert!(dir.join("identity.bin").exists());
        assert!(dir.join(PAIRING_MODEL_FILE).exists());
        assert!(authorize(&dir, &id, "relay", false).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
