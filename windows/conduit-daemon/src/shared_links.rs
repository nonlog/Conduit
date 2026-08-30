//! Bounded desktop history for explicit web-page shares from the phone.
//!
//! The file is intentionally tiny and human-readable. A share is already a user-driven event, so
//! reading/re-writing at most 100 rows costs no idle wakeups, watcher or background maintenance.

#![allow(dead_code)]

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const FILE: &str = "shared-links.tsv";
pub const MAX_ENTRIES: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub timestamp_ms: u64,
    pub url: String,
    pub title: String,
    pub source: String,
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

pub fn record(dir: &Path, url: &str, title: &str, source: &str, timestamp_ms: u64) -> Result<()> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    if url.is_empty()
        || url.len() > 4096
        || url.chars().any(char::is_control)
        || (!lower.starts_with("http://") && !lower.starts_with("https://"))
    {
        anyhow::bail!("refusing non-web or unbounded shared URL");
    }

    let entry = Entry {
        timestamp_ms: if timestamp_ms == 0 {
            now_ms()
        } else {
            timestamp_ms
        },
        url: url.to_owned(),
        title: one_line(title, 512),
        source: one_line(source, 64),
    };
    let mut entries = read(dir);
    // Re-sharing the same page makes it recent instead of filling the bounded list with duplicates.
    entries.retain(|old| old.url != entry.url);
    entries.insert(0, entry);
    entries.truncate(MAX_ENTRIES);

    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut body = String::new();
    for entry in entries {
        use std::fmt::Write as _;
        let _ = writeln!(
            body,
            "{}\t{}\t{}\t{}",
            entry.timestamp_ms, entry.url, entry.title, entry.source
        );
    }
    std::fs::write(path(dir), body).context("writing shared-link history")
}

pub fn clear(dir: &Path) -> Result<()> {
    match std::fs::remove_file(path(dir)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("clearing shared-link history"),
    }
}

fn parse_line(line: &str) -> Option<Entry> {
    let mut fields = line.splitn(4, '\t');
    Some(Entry {
        timestamp_ms: fields.next()?.parse().ok()?,
        url: fields.next()?.to_owned(),
        title: fields.next().unwrap_or_default().to_owned(),
        source: fields.next().unwrap_or_default().to_owned(),
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
            "conduit-shared-links-{name}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn history_is_newest_first_deduplicated_and_sanitised() {
        let dir = scratch("record");
        record(&dir, "https://one.example/", "One\npage", "Phone\tA", 10).unwrap();
        record(&dir, "https://two.example/", "Two", "Phone", 20).unwrap();
        record(&dir, "https://one.example/", "One again", "Phone", 30).unwrap();
        let entries = read(&dir);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].url, "https://one.example/");
        assert_eq!(entries[0].title, "One again");
        assert_eq!(entries[1].url, "https://two.example/");
        assert!(!entries
            .iter()
            .any(|entry| entry.title.contains('\n') || entry.source.contains('\t')));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unsafe_schemes_are_not_written() {
        let dir = scratch("unsafe");
        assert!(record(&dir, "file:///C:/secret.txt", "x", "phone", 1).is_err());
        assert!(read(&dir).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
