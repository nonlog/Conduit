//! One-peer trust and explicit pairing state for the Windows companion.
//!
//! Conduit deliberately keeps the current one-phone model: one persisted peer id, one display
//! name, and a short user-opened pairing window. Nothing here polls. The pairing window is just an
//! expiry timestamp checked when a real handshake arrives or when the UI asks for status.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PEER_FILE: &str = "peer.txt";
const PEER_NAME_FILE: &str = "peer-name.txt";
const PAIRING_FILE: &str = "pairing.txt";
const ID_LEN: usize = 43;
pub const WINDOW: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct State {
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub pairing: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Authorization {
    Trusted,
    Paired,
    LegacyRelayMigration,
}

pub fn state(dir: &Path) -> State {
    State {
        device_id: peer_id(dir),
        device_name: peer_name(dir),
        pairing: pairing_allowed(dir),
    }
}

pub fn peer_id(dir: &Path) -> Option<String> {
    read_one_line(&dir.join(PEER_FILE)).filter(|value| valid_id(value))
}

pub fn peer_name(dir: &Path) -> Option<String> {
    read_one_line(&dir.join(PEER_NAME_FILE)).filter(|value| !value.is_empty())
}

pub fn start(dir: &Path) -> Result<u128> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let expires = now_ms().saturating_add(WINDOW.as_millis());
    atomic_write(&dir.join(PAIRING_FILE), &format!("{expires}\n"))?;
    Ok(expires)
}

pub fn cancel(dir: &Path) -> Result<bool> {
    remove_if_exists(&dir.join(PAIRING_FILE))
}

pub fn forget(dir: &Path) -> Result<bool> {
    let mut changed = false;
    changed |= remove_if_exists(&dir.join(PEER_FILE))?;
    changed |= remove_if_exists(&dir.join(PEER_NAME_FILE))?;
    changed |= remove_if_exists(&dir.join(PAIRING_FILE))?;
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
/// A different LAN peer is accepted only while the user has explicitly opened pairing. Existing
/// installations get one compatibility path: before this feature there was no Windows-side peer
/// store, but a previously paired phone already knows this desktop's 256-bit rendezvous id and can
/// therefore arrive through Relay. The first such Relay arrival is pinned, making upgrades
/// seamless without turning fresh LAN discovery into implicit pairing.
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
    let local_pairing = pairing_allowed(dir);
    if local_pairing {
        if via != "lan" {
            bail!("pairing is LAN-only; connect both devices to the same network");
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
    if remembered.is_none() && via == "relay" {
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
    if pairing_allowed(dir) {
        let _ = cancel(dir);
    }
    remember_name(dir, name)
}

fn remember_peer(dir: &Path, remote_id: &str) -> Result<()> {
    let previous = peer_id(dir);
    atomic_write(&dir.join(PEER_FILE), &(remote_id.to_owned() + "\n"))?;
    if previous.as_deref() != Some(remote_id) {
        let _ = remove_if_exists(&dir.join(PEER_NAME_FILE));
    }
    Ok(())
}

fn pairing_allowed(dir: &Path) -> bool {
    let path = dir.join(PAIRING_FILE);
    let Some(value) = read_one_line(&path) else {
        return false;
    };
    let Ok(expires) = value.parse::<u128>() else {
        let _ = std::fs::remove_file(path);
        return false;
    };
    if expires > now_ms() {
        true
    } else {
        let _ = std::fs::remove_file(path);
        false
    }
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
    fn explicit_window_pairs_once_then_pins() {
        let dir = temp("window");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = "A".repeat(ID_LEN);
        let b = "B".repeat(ID_LEN);
        start(&dir).unwrap();
        let authorization = authorize(&dir, &a, "lan", true).unwrap();
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
        let _ = std::fs::remove_dir_all(&dir);
    }
}
