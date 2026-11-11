//! Event-driven desktop status for the on-demand control surface.
//!
//! No watcher and no timer exists here. The daemon rewrites these two tiny files only when a
//! session transitions, the peer supplies its display name, or the paired-device registry changes:
//! `status.txt` is the single-peer compatibility snapshot an older on-demand UI reads, and
//! `peers.tsv` is the per-peer view the multi-device UI reads.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::pairing;

const FILE: &str = "status.txt";
const PEERS_FILE: &str = "peers.tsv";

#[derive(Clone, Debug, Default)]
struct PeerRow {
    id: String,
    name: String,
    kind: &'static str,
    enabled: bool,
    selected: bool,
    /// `connected`, `connecting`, `paused` or `offline`; `paused` is derived when link is off.
    state: &'static str,
    /// `lan`, `relay` or empty while disconnected.
    route: &'static str,
    relay: String,
    /// What the paired phone last reported for this peer's own route to the phone.
    phone_route: String,
}

#[derive(Clone, Debug, Default)]
struct Snapshot {
    state: String,
    paused: bool,
    peer_name: String,
    peer_id: String,
    path: String,
    relay: String,
    peers: Vec<PeerRow>,
}

pub struct StatusFile {
    pub transfers: std::sync::Arc<crate::transfer::Transfers>,
    dir: PathBuf,
    path: PathBuf,
    current: Mutex<Snapshot>,
}

impl StatusFile {
    pub fn new(dir: &Path) -> Result<Self> {
        let status = Self {
            transfers: crate::transfer::Transfers::start(dir),
            dir: dir.to_path_buf(),
            path: dir.join(FILE),
            current: Mutex::new(Snapshot {
                state: "disconnected".into(),
                ..Default::default()
            }),
        };
        status.write()?;
        Ok(status)
    }

    /// Replaces the registry part of the view. Live transport state of peers that are still
    /// listed is kept, so a registry refresh does not flicker every connected row.
    pub fn set_peers(&self, peers: &[pairing::PairedPeer], selected: Option<&str>) {
        self.update(|snapshot| {
            let live = std::mem::take(&mut snapshot.peers);
            snapshot.peers = peers
                .iter()
                .map(|peer| {
                    let previous = live.iter().find(|row| row.id == peer.id);
                    PeerRow {
                        id: peer.id.clone(),
                        name: peer.name.clone(),
                        kind: peer.kind.as_str(),
                        enabled: peer.enabled,
                        selected: selected == Some(peer.id.as_str()),
                        state: previous.map_or("offline", |row| row.state),
                        route: previous.map_or("", |row| row.route),
                        relay: previous.map_or_else(String::new, |row| row.relay.clone()),
                        phone_route: previous
                            .map_or_else(String::new, |row| row.phone_route.clone()),
                    }
                })
                .collect();
        });
    }

    pub fn linked(&self, peer_id: &str, path: &str, relay: Option<&str>) {
        self.update(|snapshot| {
            if snapshot.paused {
                return;
            }
            snapshot.state = "linked".into();
            snapshot.peer_id = peer_id.into();
            snapshot.peer_name.clear();
            snapshot.path = path.into();
            snapshot.relay = relay.unwrap_or("").into();
            if let Some(row) = snapshot.peers.iter_mut().find(|row| row.id == peer_id) {
                row.state = "connected";
                row.route = if path.starts_with("relay") {
                    "relay"
                } else {
                    "lan"
                };
                row.relay = relay.unwrap_or("").into();
                if row.kind == "phone" {
                    row.phone_route = row.route.into();
                }
            }
        });
    }

    /// A dial that has started but not yet authenticated, so the UI can show intent without
    /// pretending the peer is reachable.
    pub fn connecting(&self, peer_id: &str) {
        self.update(|snapshot| {
            if snapshot.paused {
                return;
            }
            if let Some(row) = snapshot.peers.iter_mut().find(|row| row.id == peer_id) {
                if row.state != "connected" {
                    row.state = "connecting";
                }
            }
        });
    }

    /// One peer's session ended. Other peers, including their in-flight transfers, are untouched.
    pub fn disconnected(&self, peer_id: &str) {
        self.update(|snapshot| {
            let was_phone = snapshot
                .peers
                .iter()
                .find(|row| row.id == peer_id)
                .is_some_and(|row| row.kind == "phone");
            if let Some(row) = snapshot.peers.iter_mut().find(|row| row.id == peer_id) {
                row.state = if snapshot.paused { "paused" } else { "offline" };
                row.route = "";
                row.relay.clear();
            }
            if was_phone {
                // The phone's transport to this PC is the only thing `phone_route` describes.
                for row in &mut snapshot.peers {
                    row.phone_route.clear();
                }
            }
            if snapshot.peer_id == peer_id {
                snapshot.state = if snapshot.paused {
                    "paused"
                } else {
                    "disconnected"
                }
                .into();
                snapshot.peer_name.clear();
                snapshot.peer_id.clear();
                snapshot.path.clear();
                snapshot.relay.clear();
            }
        });
    }

    /// The paired phone's view of its paired devices. Only the phone may send this, and the main
    /// loop has already checked that; a roster entry never invents a local trust decision.
    pub fn phone_roster(&self, roster: &crate::wire::pb::PeerRoster) {
        self.update(|snapshot| {
            for entry in &roster.peers {
                let Some(row) = snapshot
                    .peers
                    .iter_mut()
                    .find(|row| row.id == entry.device_id)
                else {
                    continue;
                };
                row.phone_route = if entry.paused {
                    "paused".into()
                } else if !entry.online {
                    String::new()
                } else if entry.route.eq_ignore_ascii_case("lan") {
                    "lan".into()
                } else if entry.route.eq_ignore_ascii_case("relay") {
                    "relay".into()
                } else {
                    "online".into()
                };
            }
        });
    }

    pub fn peer_name(&self, name: &str) {
        self.update(|snapshot| {
            if !snapshot.paused {
                snapshot.peer_name = name.into();
            }
        });
    }

    pub fn set_link_enabled(&self, enabled: bool) {
        self.update(|snapshot| {
            snapshot.paused = !enabled;
            snapshot.state = if enabled { "disconnected" } else { "paused" }.into();
            snapshot.peer_name.clear();
            snapshot.peer_id.clear();
            snapshot.path.clear();
            snapshot.relay.clear();
            for row in &mut snapshot.peers {
                row.route = "";
                row.relay.clear();
                row.phone_route.clear();
                row.state = if enabled { "offline" } else { "paused" };
            }
        });
    }

    fn update(&self, change: impl FnOnce(&mut Snapshot)) {
        if let Ok(mut current) = self.current.lock() {
            change(&mut current);
            if let Err(e) = write_snapshot(&self.path, &current) {
                tracing::warn!(error = %e, "could not update desktop status file");
            }
            if let Err(e) = write_peers(&self.dir.join(PEERS_FILE), &current) {
                tracing::warn!(error = %e, "could not update the peer status snapshot");
            }
        }
    }

    fn write(&self) -> Result<()> {
        let current = self.current.lock().expect("status mutex poisoned");
        write_snapshot(&self.path, &current)?;
        write_peers(&self.dir.join(PEERS_FILE), &current)
    }
}

fn write_snapshot(path: &Path, snapshot: &Snapshot) -> Result<()> {
    let updated_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let body = format!(
        "state={}\npeer_name={}\npeer_id={}\npath={}\nrelay={}\nupdated_ms={}\nlink_enabled={}\n",
        one_line(&snapshot.state),
        one_line(&snapshot.peer_name),
        one_line(&snapshot.peer_id),
        one_line(&snapshot.path),
        one_line(&snapshot.relay),
        updated_ms,
        !snapshot.paused,
    );
    std::fs::write(path, body).with_context(|| format!("writing {}", path.display()))
}

/// `id\tname\tkind\tstate\troute\trelay\tenabled\tselected\tphone_route`
fn write_peers(path: &Path, snapshot: &Snapshot) -> Result<()> {
    let body: String = snapshot
        .peers
        .iter()
        .map(|row| {
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                row.id,
                one_line(&row.name),
                row.kind,
                if snapshot.paused { "paused" } else { row.state },
                row.route,
                one_line(&row.relay),
                u8::from(row.enabled),
                u8::from(row.selected),
                row.phone_route,
            )
        })
        .collect();
    // Same crash-safe pattern as the transfer history: a reader never sees a half-written row.
    let temp = path.with_extension("tsv.tmp");
    std::fs::write(&temp, body).with_context(|| format!("writing {}", temp.display()))?;
    std::fs::rename(&temp, path)
        .with_context(|| format!("moving {} to {}", temp.display(), path.display()))
}

fn one_line(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

pub fn read(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join(FILE)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "conduit-status-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read_peers(dir: &Path) -> String {
        std::fs::read_to_string(dir.join(PEERS_FILE)).unwrap()
    }

    fn peer(id: &str, name: &str, kind: pairing::PeerKind) -> pairing::PairedPeer {
        pairing::PairedPeer {
            id: id.to_owned(),
            name: name.to_owned(),
            kind,
            enabled: true,
            source: pairing::PeerSource::Direct,
        }
    }

    #[tokio::test]
    async fn status_is_one_small_event_written_snapshot() {
        let dir = temp("single");
        let status = StatusFile::new(&dir).unwrap();
        status.linked("peer-id", "relay", Some("tyo.example:41113"));
        status.peer_name("Phone\nName");
        let body = read(&dir).unwrap();
        assert!(body.contains("state=linked\n"));
        assert!(body.contains("peer_name=Phone Name\n"));
        assert!(body.contains("relay=tyo.example:41113\n"));
        status.disconnected("peer-id");
        assert!(read(&dir).unwrap().contains("state=disconnected\n"));
        status.set_link_enabled(false);
        status.disconnected("peer-id"); // delayed socket destruction cannot clear user intent
        status.linked("late", "lan", None);
        assert!(read(&dir).unwrap().contains("state=paused\n"));
        assert!(read(&dir).unwrap().contains("link_enabled=false\n"));
        status.set_link_enabled(true);
        status.linked("peer-id", "lan", None);
        assert!(read(&dir).unwrap().contains("state=linked\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn peers_tsv_reports_each_peer_with_its_own_route_and_flags() {
        let dir = temp("peers");
        let status = StatusFile::new(&dir).unwrap();
        let phone = peer("phone", "My\tphone", pairing::PeerKind::Phone);
        let desktop = peer("studio", "Studio", pairing::PeerKind::Desktop);
        status.set_peers(&[phone, desktop], Some("phone"));
        status.linked("phone", "relay", Some("tyo:41113"));
        status.linked("studio", "lan", None);
        let body = read_peers(&dir);
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            "phone\tMy phone\tphone\tconnected\trelay\ttyo:41113\t1\t1\trelay"
        );
        assert_eq!(
            lines[1],
            "studio\tStudio\tdesktop\tconnected\tlan\t\t1\t0\t"
        );
        // A roster updates only the reported route column, never the local transport state.
        status.phone_roster(&crate::wire::pb::PeerRoster {
            peers: vec![
                crate::wire::pb::PeerInfo {
                    device_id: "studio".into(),
                    online: true,
                    route: "LAN".into(),
                    ..Default::default()
                },
                crate::wire::pb::PeerInfo {
                    device_id: "phone".into(),
                    paused: true,
                    ..Default::default()
                },
            ],
        });
        let body = read_peers(&dir);
        let lines: Vec<&str> = body.lines().collect();
        assert!(
            lines[1].starts_with("studio\tStudio\tdesktop\tconnected\tlan\t"),
            "{body}"
        );
        assert!(lines[1].ends_with("\tlan"), "{body}");
        assert!(lines[0].ends_with("\tpaused"), "{body}");
        // A sibling the phone reports as offline keeps no reported route.
        status.phone_roster(&crate::wire::pb::PeerRoster {
            peers: vec![crate::wire::pb::PeerInfo {
                device_id: "studio".into(),
                route: "LAN".into(),
                ..Default::default()
            }],
        });
        assert!(
            read_peers(&dir).lines().nth(1).unwrap().ends_with('\t'),
            "{body}"
        );
        // The phone going away clears every reported phone route.
        status.disconnected("phone");
        let body = read_peers(&dir);
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines[0], "phone\tMy phone\tphone\toffline\t\t\t1\t1\t");
        assert_eq!(
            lines[1],
            "studio\tStudio\tdesktop\tconnected\tlan\t\t1\t0\t"
        );
        // Disabling the link pauses every row until the user resumes it.
        status.set_link_enabled(false);
        let body = read_peers(&dir);
        assert!(
            body.lines().all(|line| line.contains("\tpaused\t")),
            "{body}"
        );
        status.set_link_enabled(true);
        let body = read_peers(&dir);
        assert!(
            body.lines().all(|line| line.contains("\toffline\t")),
            "{body}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
