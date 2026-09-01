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
    /// Empty or absent means the Windows Downloads known folder.
    pub receive_dir: Option<String>,
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
        if let Some(value) = &self.receive_dir {
            body.push_str("receive_dir=");
            body.push_str(value);
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
        let relays = split_relays(&source);
        if legacy_default_relays(&relays) {
            split_relays(default)
        } else {
            relays
        }
    }

    pub fn resolved_proxy(&self) -> Option<String> {
        let value = std::env::var("CONDUIT_RELAY_PROXY")
            .ok()
            .or_else(|| self.relay_proxy.clone())?;
        let value = value.trim();
        if value.is_empty() {
            None
        } else if value.eq_ignore_ascii_case("system") {
            system_socks5_proxy()
        } else {
            Some(value.to_owned())
        }
    }

    pub fn show_tray_icon(&self) -> bool {
        self.tray_icon.unwrap_or(true)
    }
}

fn legacy_default_relays(relays: &[String]) -> bool {
    const LEGACY: [&str; 3] = [
        "us.414222.xyz:41113",
        "tyo.414222.xyz:41113",
        "wa.414222.xyz:41113",
    ];
    relays.len() == LEGACY.len()
        && LEGACY.iter().all(|endpoint| {
            relays
                .iter()
                .any(|value| value.eq_ignore_ascii_case(endpoint))
        })
}

fn system_socks5_proxy() -> Option<String> {
    let key = windows_registry::CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .ok()?;
    if key.get_u32("ProxyEnable").ok().unwrap_or(0) == 0 {
        return None;
    }
    let server = key.get_string("ProxyServer").ok()?;
    parse_system_proxy_server(&server)
}

fn parse_system_proxy_server(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let endpoint = if value.contains('=') {
        value.split(';').find_map(|part| {
            let (scheme, endpoint) = part.trim().split_once('=')?;
            matches!(
                scheme.trim().to_ascii_lowercase().as_str(),
                "socks" | "socks5"
            )
            .then_some(endpoint.trim())
        })?
    } else {
        value
    };
    if endpoint.is_empty() {
        return None;
    }
    Some(if endpoint.starts_with("socks5://") {
        endpoint.to_owned()
    } else {
        format!("socks5://{endpoint}")
    })
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
            "receive_dir" => config.receive_dir = Some(value),
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
        assert_eq!(
            parse("receive_dir=D:\\Received\n")
                .unwrap()
                .receive_dir
                .as_deref(),
            Some(r"D:\Received")
        );
        assert!(parse("mystery=yes\n").is_err());
        assert_eq!(parse("relays=\n").unwrap().relays.as_deref(), Some(""));
    }

    #[test]
    fn relay_list_is_ordered_and_deduplicated() {
        assert_eq!(split_relays(" wa:1;tyo:2, wa:1 "), vec!["wa:1", "tyo:2"]);
    }

    #[test]
    fn legacy_three_node_fleet_migrates_to_the_managed_default() {
        let config = Config {
            relays: Some("wa.414222.xyz:41113;us.414222.xyz:41113;tyo.414222.xyz:41113".into()),
            ..Config::default()
        };
        assert_eq!(
            config.resolved_relays(
                "conduit-us.414222.xyz:41113;conduit-wa.414222.xyz:41113;conduit-tyo.414222.xyz:41113;conduit-jp.414222.xyz:41113"
            ),
            vec![
                "conduit-us.414222.xyz:41113",
                "conduit-wa.414222.xyz:41113",
                "conduit-tyo.414222.xyz:41113",
                "conduit-jp.414222.xyz:41113",
            ]
        );
    }

    #[test]
    fn windows_proxy_strings_choose_socks_without_guessing_http_ports() {
        assert_eq!(
            parse_system_proxy_server("127.0.0.1:7890").as_deref(),
            Some("socks5://127.0.0.1:7890")
        );
        assert_eq!(
            parse_system_proxy_server("http=127.0.0.1:8080;socks=127.0.0.1:1080").as_deref(),
            Some("socks5://127.0.0.1:1080")
        );
        assert_eq!(parse_system_proxy_server("http=127.0.0.1:8080"), None);
        assert_eq!(parse_system_proxy_server(""), None);
    }
}
