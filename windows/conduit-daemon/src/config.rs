//! Tiny user-owned daemon configuration.
//!
//! No watcher, no reload thread and no registry maze: the daemon reads this once at startup.
//! Environment variables remain explicit development/backward-compatible overrides.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

const FILE: &str = "config.txt";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// `None`: not configured in the file. `Some("")`: explicitly disable Relay parking.
    pub relays: Option<String>,
    /// Empty means direct Relay dial.
    pub relay_proxy: Option<String>,
    /// `None` preserves the product default (shown). The tray is event-driven and adds no poll.
    pub tray_icon: Option<bool>,
}

impl Config {
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        parse(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self, dir: &Path) -> Result<PathBuf> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.join(FILE);
        let mut body = String::new();
        if let Some(value) = &self.relays {
            body.push_str("relays=");
            body.push_str(value);
            body.push('\n');
        }
        if let Some(value) = &self.relay_proxy {
            body.push_str("relay_proxy=");
            body.push_str(value);
            body.push('\n');
        }
        if let Some(value) = self.tray_icon {
            body.push_str("tray_icon=");
            body.push_str(if value { "true" } else { "false" });
            body.push('\n');
        }
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        Ok(path)
    }

    pub fn resolved_relays(&self, default: &str) -> Vec<String> {
        let source = match std::env::var("CONDUIT_RELAYS") {
            Ok(value) => value,
            Err(_) => match &self.relays {
                Some(value) => value.clone(),
                None => std::env::var("CONDUIT_RELAY").unwrap_or_else(|_| default.to_string()),
            },
        };
        split_relays(&source)
    }

    pub fn resolved_proxy(&self) -> Option<String> {
        let value = std::env::var("CONDUIT_RELAY_PROXY")
            .ok()
            .or_else(|| self.relay_proxy.clone())?;
        let value = value.trim().to_owned();
        (!value.is_empty()).then_some(value)
    }

    pub fn show_tray_icon(&self) -> bool {
        self.tray_icon.unwrap_or(true)
    }
}

fn parse(text: &str) -> Result<Config> {
    let mut config = Config::default();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .with_context(|| format!("line {} has no '='", index + 1))?;
        let value = value.trim().to_string();
        match key.trim() {
            "relays" => config.relays = Some(value),
            "relay_proxy" => config.relay_proxy = Some(value),
            "tray_icon" => {
                config.tray_icon = Some(match value.to_ascii_lowercase().as_str() {
                    "true" | "1" | "yes" | "on" => true,
                    "false" | "0" | "no" | "off" => false,
                    _ => bail!("invalid tray_icon value {value:?} on line {}", index + 1),
                })
            }
            other => bail!("unknown configuration key {other:?} on line {}", index + 1),
        }
    }
    Ok(config)
}

fn split_relays(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for endpoint in source
        .split([',', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if !out.iter().any(|seen| seen == endpoint) {
            out.push(endpoint.to_owned());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_parser_is_small_strict_and_preserves_explicit_empty_values() {
        let config = parse(
            "# Conduit\nrelays=wa.example:41113;tyo.example:41113\nrelay_proxy=socks5://127.0.0.1:7891\n",
        )
        .unwrap();
        assert_eq!(
            config.relays.as_deref(),
            Some("wa.example:41113;tyo.example:41113")
        );
        assert_eq!(
            config.relay_proxy.as_deref(),
            Some("socks5://127.0.0.1:7891")
        );
        assert!(config.show_tray_icon());
        assert!(!parse("tray_icon=false\n").unwrap().show_tray_icon());
        assert!(parse("mystery=yes\n").is_err());
        assert_eq!(parse("relays=\n").unwrap().relays.as_deref(), Some(""));
    }

    #[test]
    fn relay_list_is_ordered_and_deduplicated() {
        assert_eq!(split_relays(" wa:1;tyo:2, wa:1 "), vec!["wa:1", "tyo:2"]);
    }
}
