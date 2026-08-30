//! Bounded local history for notifications mirrored from Android.
//!
//! Notification traffic is already event-driven, so this file is touched only when a notification
//! arrives or changes. Keeping at most 100 rows makes the UI useful without adding a watcher,
//! polling loop, database, or long-lived worker.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const FILE: &str = "notifications.tsv";
pub const MAX_ENTRIES: usize = 100;

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
    entries.retain(|old| old.key != entry.key);
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
    std::fs::write(path(dir), body).context("writing notification history")
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
    fn bounded_history_deduplicates_by_notification_key() {
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
