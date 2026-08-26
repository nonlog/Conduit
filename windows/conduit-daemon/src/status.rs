//! Event-driven desktop status for the on-demand control surface.
//!
//! No watcher and no timer exists here. The daemon rewrites this tiny file only when a session
//! transitions or the peer supplies its display name; an on-demand UI can then read it once.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const FILE: &str = "status.txt";

#[derive(Clone, Debug, Default)]
struct Snapshot {
    state: String,
    peer_name: String,
    peer_id: String,
    path: String,
    relay: String,
}

pub struct StatusFile {
    path: PathBuf,
    current: Mutex<Snapshot>,
}

impl StatusFile {
    pub fn new(dir: &Path) -> Result<Self> {
        let status = Self {
            path: dir.join(FILE),
            current: Mutex::new(Snapshot {
                state: "disconnected".into(),
                ..Default::default()
            }),
        };
        status.write()?;
        Ok(status)
    }

    pub fn linked(&self, peer_id: &str, path: &str, relay: Option<&str>) {
        self.update(|snapshot| {
            snapshot.state = "linked".into();
            snapshot.peer_id = peer_id.into();
            snapshot.peer_name.clear();
            snapshot.path = path.into();
            snapshot.relay = relay.unwrap_or("").into();
        });
    }

    pub fn peer_name(&self, name: &str) {
        self.update(|snapshot| snapshot.peer_name = name.into());
    }

    pub fn disconnected(&self) {
        self.update(|snapshot| {
            snapshot.state = "disconnected".into();
            snapshot.peer_name.clear();
            snapshot.peer_id.clear();
            snapshot.path.clear();
            snapshot.relay.clear();
        });
    }

    fn update(&self, change: impl FnOnce(&mut Snapshot)) {
        if let Ok(mut current) = self.current.lock() {
            change(&mut current);
            if let Err(e) = write_snapshot(&self.path, &current) {
                tracing::warn!(error = %e, "could not update desktop status file");
            }
        }
    }

    fn write(&self) -> Result<()> {
        let current = self.current.lock().expect("status mutex poisoned");
        write_snapshot(&self.path, &current)
    }
}

fn write_snapshot(path: &Path, snapshot: &Snapshot) -> Result<()> {
    let updated_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let body = format!(
        "state={}\npeer_name={}\npeer_id={}\npath={}\nrelay={}\nupdated_ms={}\n",
        one_line(&snapshot.state),
        one_line(&snapshot.peer_name),
        one_line(&snapshot.peer_id),
        one_line(&snapshot.path),
        one_line(&snapshot.relay),
        updated_ms,
    );
    std::fs::write(path, body).with_context(|| format!("writing {}", path.display()))
}

fn one_line(value: &str) -> String {
    value.replace(['\r', '\n'], " ")
}

pub fn read(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join(FILE)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_is_one_small_event_written_snapshot() {
        let dir = std::env::temp_dir().join(format!("conduit-status-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let status = StatusFile::new(&dir).unwrap();
        status.linked("peer-id", "relay", Some("tyo.example:41113"));
        status.peer_name("Phone\nName");
        let body = read(&dir).unwrap();
        assert!(body.contains("state=linked\n"));
        assert!(body.contains("peer_name=Phone Name\n"));
        assert!(body.contains("relay=tyo.example:41113\n"));
        status.disconnected();
        assert!(read(&dir).unwrap().contains("state=disconnected\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
