//! Bounded local history for notifications mirrored from Android.
//!
//! Notification traffic is already event-driven, so this file is touched only when a notification
//! arrives or changes. Keeping at most 100 rows makes the UI useful without adding a watcher,
//! polling loop, database, or long-lived worker.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Notify;

const FILE: &str = "notifications.tsv";
pub const MAX_ENTRIES: usize = 100;
pub const DEFAULT_RETENTION_DAYS: u32 = 30;
const DAY_MS: u64 = 86_400_000;

/// One serialized owner for history writes, settings changes and explicit clears.
/// The maintenance task sleeps until the oldest retained row expires, not on a poll.
pub struct History {
    dir: PathBuf,
    days: Mutex<u32>,
    changed: Notify,
}

impl History {
    pub fn new(dir: &Path, days: u32) -> Result<Arc<Self>> {
        anyhow::ensure!(days <= 3650, "notification retention must be 0..3650 days");
        let history = Arc::new(Self {
            dir: dir.to_owned(),
            days: Mutex::new(days),
            changed: Notify::new(),
        });
        history.prune()?;
        Ok(history)
    }

    pub fn configure(&self, days: u32) -> Result<()> {
        anyhow::ensure!(days <= 3650, "notification retention must be 0..3650 days");
        let mut policy = self.days.lock().unwrap_or_else(|error| error.into_inner());
        prune_at(&self.dir, days, now_ms())?;
        *policy = days;
        self.changed.notify_one();
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        let _guard = self.days.lock().unwrap_or_else(|error| error.into_inner());
        write(&self.dir, Vec::new())?;
        self.changed.notify_one();
        Ok(())
    }

    pub fn prune(&self) -> Result<usize> {
        let days = self.days.lock().unwrap_or_else(|error| error.into_inner());
        let removed = prune_at(&self.dir, *days, now_ms())?;
        self.changed.notify_one();
        Ok(removed)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_new(
        &self,
        key: &str,
        package: &str,
        app: &str,
        title: &str,
        body: &str,
        timestamp_ms: u64,
    ) -> Result<()> {
        let days = self.days.lock().unwrap_or_else(|error| error.into_inner());
        let now = now_ms();
        // A bad remote clock must not make a notification survive forever.
        let timestamp = if timestamp_ms == 0 {
            now
        } else {
            timestamp_ms.min(now)
        };
        record_new(&self.dir, key, package, app, title, body, timestamp)?;
        prune_at(&self.dir, *days, now)?;
        self.changed.notify_one();
        Ok(())
    }

    pub fn record_update(&self, key: &str, title: &str, body: &str) -> Result<()> {
        let days = self.days.lock().unwrap_or_else(|error| error.into_inner());
        record_update(&self.dir, key, title, body)?;
        prune_at(&self.dir, *days, now_ms())?;
        self.changed.notify_one();
        Ok(())
    }

    pub async fn maintain(self: Arc<Self>) {
        loop {
            let changed = self.changed.notified();
            let expiry = {
                let days = self.days.lock().unwrap_or_else(|error| error.into_inner());
                next_expiry(&read(&self.dir), *days)
            };
            match expiry {
                None => changed.await,
                Some(expiry) => {
                    let delay =
                        std::time::Duration::from_millis(expiry.saturating_sub(now_ms()).max(1));
                    tokio::select! {
                        _ = changed => {},
                        _ = tokio::time::sleep(delay) => {
                            if let Err(error) = self.prune() {
                                tracing::warn!(%error, "notification expiry cleanup failed; retry on next history event");
                                // A broken filesystem must not produce an idle retry loop.
                                self.changed.notified().await;
                            }
                        }
                    }
                }
            }
        }
    }
}

fn expired(timestamp_ms: u64, days: u32, now: u64) -> bool {
    days != 0 && timestamp_ms < now.saturating_sub(u64::from(days) * DAY_MS)
}

fn next_expiry(entries: &[Entry], days: u32) -> Option<u64> {
    if days == 0 {
        return None;
    }
    entries
        .iter()
        .map(|entry| {
            entry
                .timestamp_ms
                .saturating_add(u64::from(days) * DAY_MS)
                .saturating_add(1)
        })
        .min()
}

fn prune_at(dir: &Path, days: u32, now: u64) -> Result<usize> {
    let mut entries = read(dir);
    let before = entries.len();
    entries.retain(|entry| !expired(entry.timestamp_ms, days, now));
    let removed = before - entries.len();
    if removed != 0 {
        write(dir, entries)?;
    }
    Ok(removed)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub timestamp_ms: u64,
    pub key: String,
    pub package: String,
    pub app: String,
    pub title: String,
    pub body: String,
}

pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

pub fn read(dir: &Path) -> Vec<Entry> {
    let Ok(body) = std::fs::read_to_string(path(dir)) else {
        return Vec::new();
    };
    body.lines()
        .filter_map(parse_line)
        .take(MAX_ENTRIES)
        .collect()
}

pub fn record_new(
    dir: &Path,
    key: &str,
    package: &str,
    app: &str,
    title: &str,
    body: &str,
    timestamp_ms: u64,
) -> Result<()> {
    let key = one_line(key, 512);
    if key.is_empty() {
        anyhow::bail!("notification has no key");
    }
    let entry = Entry {
        timestamp_ms: if timestamp_ms == 0 {
            now_ms()
        } else {
            timestamp_ms
        },
        key,
        package: one_line(package, 256),
        app: one_line(app, 128),
        title: one_line(title, 512),
        body: one_line(body, 1024),
    };
    let mut entries = read(dir);
    // A single Android notification key can represent multiple real arrivals (chat apps commonly
    // reuse one conversation key). History is an event log, so every NOTIF_NEW keeps its own row.
    entries.insert(0, entry);
    write(dir, entries)
}

pub fn record_update(dir: &Path, key: &str, title: &str, body: &str) -> Result<()> {
    let key = one_line(key, 512);
    if key.is_empty() {
        anyhow::bail!("notification update has no key");
    }
    let mut entries = read(dir);
    let existing = entries.iter().position(|entry| entry.key == key);
    let mut entry = existing
        .map(|index| entries.remove(index))
        .unwrap_or_else(|| Entry {
            timestamp_ms: now_ms(),
            key: key.clone(),
            package: String::new(),
            app: String::from("Phone"),
            title: String::new(),
            body: String::new(),
        });
    entry.timestamp_ms = now_ms();
    entry.title = one_line(title, 512);
    entry.body = one_line(body, 1024);
    entries.insert(0, entry);
    write(dir, entries)
}

fn write(dir: &Path, mut entries: Vec<Entry>) -> Result<()> {
    entries.truncate(MAX_ENTRIES);
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut body = String::new();
    for entry in entries {
        use std::fmt::Write as _;
        let _ = writeln!(
            body,
            "{}\t{}\t{}\t{}\t{}\t{}",
            entry.timestamp_ms, entry.key, entry.package, entry.app, entry.title, entry.body,
        );
    }
    let destination = path(dir);
    let temporary = destination.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, body).context("writing notification history")?;
    std::fs::rename(&temporary, destination).context("publishing notification history")
}

fn parse_line(line: &str) -> Option<Entry> {
    let mut fields = line.splitn(6, '\t');
    Some(Entry {
        timestamp_ms: fields.next()?.parse().ok()?,
        key: fields.next()?.to_owned(),
        package: fields.next().unwrap_or_default().to_owned(),
        app: fields.next().unwrap_or_default().to_owned(),
        title: fields.next().unwrap_or_default().to_owned(),
        body: fields.next().unwrap_or_default().to_owned(),
    })
}

fn one_line(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .map(|ch| {
            if ch == '\r' || ch == '\n' || ch == '\t' {
                ' '
            } else {
                ch
            }
        })
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_boundary_disabled_policy_and_future_timestamps_are_explicit() {
        let now = 90 * DAY_MS;
        assert!(!expired(now - 30 * DAY_MS, 30, now));
        assert!(expired(now - 30 * DAY_MS - 1, 30, now));
        assert!(!expired(1, 0, now));
        assert!(!expired(now + DAY_MS, 30, now));
        assert_eq!(next_expiry(&[], 30), None);
    }

    #[test]
    fn retention_and_clear_serialize_without_rewriting_pairing() {
        let dir = scratch("policy");
        std::fs::write(dir.join("identity.bin"), b"preserve").unwrap();
        record_new(&dir, "old", "pkg", "App", "Old", "Body", 1).unwrap();
        record_new(&dir, "new", "pkg", "App", "Recent", "Body", now_ms()).unwrap();
        let history = History::new(&dir, 30).unwrap();
        assert_eq!(read(&dir).len(), 1);
        history.configure(0).unwrap();
        history
            .record_new("keep", "pkg", "App", "Old", "Body", 1)
            .unwrap();
        assert_eq!(read(&dir).len(), 2);
        assert!(history.configure(3651).is_err());
        history.clear().unwrap();
        assert!(read(&dir).is_empty());
        assert_eq!(
            std::fs::read(dir.join("identity.bin")).unwrap(),
            b"preserve"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "conduit-notifications-{name}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn new_and_update_keep_one_recent_sanitised_entry() {
        let dir = scratch("update");
        record_new(
            &dir,
            "key-1",
            "com.example.chat",
            "Chat\tApp",
            "Old\nTitle",
            "Old body",
            10,
        )
        .unwrap();
        record_update(&dir, "key-1", "New title", "New\tbody").unwrap();
        let entries = read(&dir);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].app, "Chat App");
        assert_eq!(entries[0].title, "New title");
        assert_eq!(entries[0].body, "New body");
        assert!(entries[0].timestamp_ms >= 10);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn repeated_key_keeps_each_new_event() {
        let dir = scratch("repeated-key");
        record_new(&dir, "same-key", "pkg", "App", "First", "one", 10).unwrap();
        record_new(&dir, "same-key", "pkg", "App", "Second", "two", 20).unwrap();
        let entries = read(&dir);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "Second");
        assert_eq!(entries[1].title, "First");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bounded_history_keeps_newest_entries() {
        let dir = scratch("bounded");
        for index in 0..(MAX_ENTRIES + 20) {
            record_new(
                &dir,
                &format!("key-{index}"),
                "pkg",
                "App",
                "Title",
                "Body",
                index as u64 + 1,
            )
            .unwrap();
        }
        let entries = read(&dir);
        assert_eq!(entries.len(), MAX_ENTRIES);
        assert_eq!(entries[0].key, format!("key-{}", MAX_ENTRIES + 19));
        let _ = std::fs::remove_dir_all(dir);
    }
}
