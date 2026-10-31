//! Boundary between a short-lived native UI and the already-resident Conduit daemon.
//! Never opens identity.bin for writing, never implements Noise or a second transport.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Connection {
    pub state: String,
    pub route: String,
    pub relay: String,
    pub device: String,
    pub linked: bool,
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Pairing {
    pub paired: bool,
    pub device: String,
    pub code: String,
    pub active: bool,
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Settings {
    pub relays: Vec<String>,
    pub proxy_mode: String,
    pub manual_proxy: String,
    pub receive_dir: String,
    pub tray_icon: bool,
    pub autostart: bool,
    pub explorer: bool,
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Notification {
    pub id: String,
    pub app: String,
    pub title: String,
    pub body: String,
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct SharedLink {
    pub id: String,
    pub title: String,
    pub url: String,
    pub source: String,
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Transfer {
    pub id: String,
    pub name: String,
    pub direction: String,
    pub bytes: u64,
    pub total: u64,
    pub state: String,
    pub detail: String,
    pub terminal: bool,
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Snapshot {
    pub connection: Connection,
    pub pairing: Pairing,
    pub settings: Settings,
    pub notifications: Vec<Notification>,
    pub links: Vec<SharedLink>,
    pub transfers: Vec<Transfer>,
    pub daemon_available: bool,
}

pub const OBSERVED_FILES: &[&str] = &[
    "status.txt",
    "config.txt",
    "pairing.txt",
    "peer.txt",
    "peer-name.txt",
    "notifications.tsv",
    "shared-links.tsv",
    "transfers.tsv",
];

pub fn wallpaper_bytes() -> Option<Vec<u8>> {
    let path = data_dir().join("wallpaper.jpg");
    let len = fs::metadata(&path).ok()?.len();
    if len == 0 || len > 1024 * 1024 {
        return None;
    }
    fs::read(path).ok()
}

fn bounded(path: &Path, max: u64) -> String {
    let Ok(file) = File::open(path) else {
        return String::new();
    };
    let mut text = String::new();
    if file.take(max).read_to_string(&mut text).is_ok() {
        text
    } else {
        String::new()
    }
}

fn kv(path: &Path) -> BTreeMap<String, String> {
    bounded(path, 128 * 1024)
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect()
}

fn get(values: &BTreeMap<String, String>, key: &str) -> String {
    values.get(key).cloned().unwrap_or_default()
}

fn scoop_root_from_exe(exe: &Path) -> Option<PathBuf> {
    let directory = exe.parent()?;
    let application = directory.parent()?;
    let apps = application.parent()?;
    if !application
        .file_name()?
        .to_string_lossy()
        .eq_ignore_ascii_case("conduit")
        || !apps
            .file_name()?
            .to_string_lossy()
            .eq_ignore_ascii_case("apps")
    {
        return None;
    }
    valid_scoop_root(apps.parent()?.to_path_buf())
}

fn valid_scoop_root(root: PathBuf) -> Option<PathBuf> {
    root.join("apps/conduit/current/conduit-daemon.exe")
        .is_file()
        .then_some(root)
}

fn scoop_root() -> Option<PathBuf> {
    if let Some(root) = env::current_exe()
        .ok()
        .as_deref()
        .and_then(scoop_root_from_exe)
    {
        return Some(root);
    }
    if let Some(root) = env::var_os("SCOOP")
        .map(PathBuf::from)
        .and_then(valid_scoop_root)
    {
        return Some(root);
    }
    if let Some(paths) = env::var_os("PATH") {
        for dir in env::split_paths(&paths) {
            if dir
                .file_name()
                .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case("shims"))
            {
                if let Some(root) = dir
                    .parent()
                    .map(Path::to_path_buf)
                    .and_then(valid_scoop_root)
                {
                    return Some(root);
                }
            }
        }
    }
    if let Some(root) = env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|dir| dir.join("scoop"))
        .and_then(valid_scoop_root)
    {
        return Some(root);
    }
    // An existing registered Conduit service can be installed on a custom Scoop drive.
    #[cfg(windows)]
    {
        use winreg::{RegKey, enums::HKEY_CURRENT_USER};
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(run) = hkcu.open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run") {
            if let Ok(command) = run.get_value::<String, _>("Conduit") {
                if let Some(root) = scoop_root_from_exe(Path::new(command.trim().trim_matches('"')))
                {
                    return Some(root);
                }
            }
        }
    }
    None
}

fn sibling_daemon() -> Option<PathBuf> {
    let sibling = env::current_exe()
        .ok()?
        .with_file_name("conduit-daemon.exe");
    sibling.is_file().then_some(sibling)
}

fn portable_daemon_for(executable: &Path) -> Option<PathBuf> {
    // A standalone ZIP must use its own daemon and pairing, even when a
    // completely unrelated Scoop installation is discoverable through PATH.
    if scoop_root_from_exe(executable).is_some() {
        return None;
    }
    let daemon = executable.with_file_name("conduit-daemon.exe");
    daemon.is_file().then_some(daemon)
}

fn portable_daemon() -> Option<PathBuf> {
    env::current_exe()
        .ok()
        .as_deref()
        .and_then(portable_daemon_for)
}

pub fn data_dir() -> PathBuf {
    if let Some(path) = env::var_os("CONDUIT_DATA_DIR").filter(|s| !s.is_empty()) {
        return PathBuf::from(path);
    }
    if let Some(daemon) = portable_daemon() {
        return daemon.parent().unwrap().join("data");
    }
    if let Some(root) = scoop_root() {
        return root.join("persist/conduit/data");
    }
    if let Some(daemon) = sibling_daemon() {
        return daemon.parent().unwrap().join("data");
    }
    env::var_os("LOCALAPPDATA")
        .or_else(|| env::var_os("APPDATA"))
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("Conduit")
}

pub fn theme_preference() -> String {
    let stored = bounded(&data_dir().join("rune-theme.txt"), 32);
    normalized_theme(&stored).into()
}

fn normalized_theme(stored: &str) -> &'static str {
    match stored.trim() {
        "system" => "system",
        "light" => "light",
        "dark" => "dark",
        // Conduit is a persistent connection utility. Its restrained dark
        // workbench is the default; explicit System/Light selections persist.
        _ => "dark",
    }
}

pub fn save_theme_preference(theme: &str) -> Result<(), String> {
    if !["system", "light", "dark"].contains(&theme) {
        return Err("Unknown appearance preference".into());
    }
    let dir = data_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(dir.join("rune-theme.txt"), theme).map_err(|e| e.to_string())
}

pub fn daemon_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os("CONDUIT_DAEMON_PATH").map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(daemon) = portable_daemon() {
        return Some(daemon);
    }
    if let Some(daemon) = sibling_daemon() {
        return Some(daemon);
    }
    scoop_root()
        .map(|root| root.join("apps/conduit/current/conduit-daemon.exe"))
        .filter(|path| path.is_file())
}

fn parse_transfer(line: &str) -> Option<Transfer> {
    let cols: Vec<_> = line.splitn(7, '\t').collect();
    if cols.len() != 7
        || !(2..=128).contains(&cols[0].len())
        || cols[0].len() % 2 != 0
        || !cols[0].bytes().all(|c| c.is_ascii_hexdigit())
        || !["inbound", "outbound"].contains(&cols[1])
        || ![
            "queued",
            "transferring",
            "waiting confirmation",
            "completed",
            "failed",
            "cancelled",
        ]
        .contains(&cols[4])
    {
        return None;
    }
    let bytes = cols[2].parse().ok()?;
    let total = cols[3].parse().ok()?;
    if bytes > total {
        return None;
    }
    Some(Transfer {
        id: cols[0].into(),
        name: cols[5].into(),
        direction: cols[1].into(),
        bytes,
        total,
        state: cols[4].into(),
        detail: cols[6].into(),
        terminal: ["completed", "failed", "cancelled"].contains(&cols[4]),
    })
}

pub fn snapshot() -> Snapshot {
    let dir = data_dir();
    let status = kv(&dir.join("status.txt"));
    let config = kv(&dir.join("config.txt"));
    let pair = kv(&dir.join("pairing.txt"));
    let device = bounded(&dir.join("peer-name.txt"), 1024).trim().to_string();
    let paired = !bounded(&dir.join("peer.txt"), 512).trim().is_empty();
    let expiry = get(&pair, "expires_ms").parse::<u64>().unwrap_or(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let code = get(&pair, "code");
    let active = expiry > now && code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit());
    let state = get(&status, "state");
    let relay_proxy = get(&config, "relay_proxy");
    let (proxy_mode, manual_proxy) = match relay_proxy.as_str() {
        "system" => ("system", ""),
        "" => ("direct", ""),
        _ => ("manual", relay_proxy.as_str()),
    };
    #[cfg(windows)]
    let (autostart, explorer) = {
        use winreg::{RegKey, enums::HKEY_CURRENT_USER};
        let registry = RegKey::predef(HKEY_CURRENT_USER);
        (
            registry
                .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")
                .and_then(|key| key.get_value::<String, _>("Conduit"))
                .is_ok(),
            registry
                .open_subkey(r"Software\Classes\*\shell\Conduit.SendToPhone")
                .is_ok(),
        )
    };
    #[cfg(not(windows))]
    let (autostart, explorer) = (false, false);
    Snapshot {
        connection: Connection {
            linked: state == "linked",
            state: match state.as_str() {
                "linked" => "Connected",
                "connecting" => "Connecting",
                "retrying" => "Reconnecting",
                _ => "Disconnected",
            }
            .into(),
            device: {
                let current = get(&status, "peer_name");
                if !current.is_empty() {
                    current
                } else if !device.is_empty() {
                    device.clone()
                } else {
                    "No phone linked".into()
                }
            },
            route: get(&status, "path"),
            relay: get(&status, "relay"),
        },
        pairing: Pairing {
            paired,
            device,
            active,
            code: if active { code } else { String::new() },
        },
        settings: Settings {
            relays: get(&config, "relays")
                .split([';', '\r', '\n', ','])
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect(),
            proxy_mode: proxy_mode.into(),
            manual_proxy: manual_proxy.into(),
            receive_dir: get(&config, "receive_dir"),
            tray_icon: config
                .get("tray_icon")
                .map(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on"))
                .unwrap_or(true),
            autostart,
            explorer,
        },
        notifications: bounded(&dir.join("notifications.tsv"), 2 * 1024 * 1024)
            .lines()
            .take(100)
            .enumerate()
            .filter_map(|(index, line)| {
                let parts: Vec<_> = line.splitn(6, '\t').collect();
                let timestamp = parts.first()?.parse::<u64>().ok()?;
                let app = parts
                    .get(3)
                    .copied()
                    .filter(|s| !s.is_empty())
                    .or_else(|| parts.get(2).copied())
                    .unwrap_or("Phone");
                Some(Notification {
                    id: format!("{timestamp}-{index}"),
                    app: app.into(),
                    title: parts
                        .get(4)
                        .copied()
                        .filter(|s| !s.is_empty())
                        .unwrap_or("Notification")
                        .into(),
                    body: parts.get(5).copied().unwrap_or("").into(),
                })
            })
            .collect(),
        links: bounded(&dir.join("shared-links.tsv"), 2 * 1024 * 1024)
            .lines()
            .take(200)
            .enumerate()
            .filter_map(|(i, line)| {
                let parts: Vec<_> = line.split('\t').collect();
                let timestamp = parts.first()?.parse::<u64>().ok()?;
                let url = parts.get(1)?.trim();
                if url.len() > 2048
                    || !url::Url::parse(url)
                        .is_ok_and(|parsed| ["http", "https"].contains(&parsed.scheme()))
                {
                    return None;
                }
                Some(SharedLink {
                    id: format!("{timestamp}-{i}"),
                    url: url.into(),
                    title: parts
                        .get(2)
                        .copied()
                        .filter(|v| !v.is_empty())
                        .unwrap_or(url)
                        .into(),
                    source: parts.get(3).copied().unwrap_or("Phone").into(),
                })
            })
            .collect(),
        transfers: bounded(&dir.join("transfers.tsv"), 2 * 1024 * 1024)
            .lines()
            .take(32)
            .filter_map(parse_transfer)
            .collect(),
        daemon_available: daemon_path().is_some(),
    }
}

fn command() -> Result<Command, String> {
    let path = daemon_path().ok_or("Cannot locate Conduit's installed daemon")?;
    let mut command = Command::new(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW, including error paths
    }
    Ok(command)
}

pub fn daemon_command(arguments: &[&str]) -> Result<String, String> {
    let result = command()?
        .args(arguments)
        .output()
        .map_err(|e| format!("Starting Conduit daemon command: {e}"))?;
    if !result.status.success() {
        let error = String::from_utf8_lossy(&result.stderr).trim().to_owned();
        return Err(if error.is_empty() {
            format!("Daemon returned {}", result.status)
        } else {
            error.trim_start_matches("Error: ").to_owned()
        });
    }
    Ok(String::from_utf8_lossy(&result.stdout).trim().to_owned())
}

pub fn daemon_running() -> bool {
    // Conduit's main.rs reserves this port for its single-instance control endpoint.
    match TcpListener::bind(("0.0.0.0", 41112)) {
        Ok(probe) => {
            drop(probe);
            false
        }
        Err(_) => true,
    }
}

pub fn ensure_daemon() -> Result<(), String> {
    if daemon_running() {
        return Ok(());
    }
    command()?
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Launching daemon: {e}"))
}

pub fn send_file(path: &Path) -> Result<(), String> {
    let metadata = fs::metadata(path).map_err(|e| format!("File unavailable: {e}"))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err("Select a nonempty file".into());
    }
    let path = fs::canonicalize(path).map_err(|e| format!("Resolving file: {e}"))?;
    let mut child = command()?
        .arg("send")
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Queueing file: {e}"))?;
    // The queue lives in the daemon; the UI remains responsive during transfers.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

pub fn cancel_transfer(id: &str) -> Result<(), String> {
    if !(2..=128).contains(&id.len())
        || id.len() % 2 != 0
        || !id.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("Invalid transfer identifier".into());
    }
    daemon_command(&["cancel-transfer", id]).map(|_| ())
}

pub fn clear_history(kind: &str) -> Result<(), String> {
    let filename = match kind {
        "notifications" => "notifications.tsv",
        "links" => "shared-links.tsv",
        _ => return Err("Invalid history category".into()),
    };
    let path = data_dir().join(filename);
    if path.is_file() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn merge_config(original: &str, updates: &[(&str, String)]) -> String {
    let replacements = updates
        .iter()
        .map(|(k, v)| (*k, v))
        .collect::<BTreeMap<_, _>>();
    let mut written = BTreeSet::new();
    let mut lines = Vec::new();
    for line in original.lines() {
        if let Some((key, _)) = line.split_once('=') {
            let key = key.trim();
            if let Some(value) = replacements.get(key) {
                if written.insert(key.to_owned()) {
                    lines.push(format!("{key}={value}"));
                }
                continue;
            }
        }
        lines.push(line.to_owned());
    }
    for (key, value) in updates {
        if !written.contains(*key) {
            lines.push(format!("{key}={value}"));
        }
    }
    format!("{}\n", lines.join("\n"))
}

fn validate(settings: &Settings) -> Result<(), String> {
    if settings.relays.len() > 12
        || !["system", "direct", "manual"].contains(&settings.proxy_mode.as_str())
    {
        return Err("Invalid number of relays or proxy mode".into());
    }
    if settings.relays.iter().any(|endpoint| {
        endpoint.len() > 255
            || !endpoint.is_ascii()
            || endpoint
                .chars()
                .any(|c| !(c.is_ascii_alphanumeric() || ".:-_".contains(c)))
    }) {
        return Err("Relay endpoints must be host:port".into());
    }
    if settings.manual_proxy.len() > 512
        || settings.manual_proxy.chars().any(char::is_control)
        || settings.receive_dir.len() > 1024
        || settings.receive_dir.chars().any(char::is_control)
    {
        return Err("Invalid proxy or receive folder".into());
    }
    Ok(())
}

pub fn save_settings(settings: Settings) -> Result<(), String> {
    validate(&settings)?;
    let dir = data_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("Creating settings directory: {e}"))?;
    let path = dir.join("config.txt");
    let original = if path.is_file() {
        let file_size = fs::metadata(&path).map_err(|e| e.to_string())?.len();
        if file_size > 128 * 1024 {
            return Err("Existing configuration is too large".into());
        }
        fs::read_to_string(&path).map_err(|e| e.to_string())?
    } else {
        String::new()
    };
    let updates = [
        ("relays", settings.relays.join(";")),
        (
            "relay_proxy",
            match settings.proxy_mode.as_str() {
                "system" => "system".into(),
                "manual" => settings.manual_proxy.trim().into(),
                _ => String::new(),
            },
        ),
        ("tray_icon", settings.tray_icon.to_string()),
        ("receive_dir", settings.receive_dir.trim().into()),
    ];
    let merged = merge_config(&original, &updates);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp = dir.join(format!("config.{}.{}.tmp", std::process::id(), stamp));
    let result = (|| -> std::io::Result<()> {
        let mut writer = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        writer.write_all(merged.as_bytes())?;
        writer.sync_all()?;
        drop(writer);
        fs::rename(&temp, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(|e| format!("Saving settings: {e}"))?;
    daemon_command(&["reload"]).map(|_| ())
}

pub fn open_external(url: &str) -> Result<(), String> {
    if url.len() > 2048
        || !url::Url::parse(url).is_ok_and(|parsed| ["http", "https"].contains(&parsed.scheme()))
    {
        return Err("Invalid web link".into());
    }
    let mut command = Command::new("explorer.exe");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn open_folder(kind: &str) -> Result<(), String> {
    let path = if kind == "data" {
        data_dir()
    } else if kind == "received" {
        let from_config = snapshot().settings.receive_dir;
        if from_config.is_empty() {
            env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join("Downloads")
        } else {
            PathBuf::from(from_config)
        }
    } else {
        return Err("Invalid folder".into());
    };
    if !path.is_dir() {
        return Err("Folder does not exist".into());
    }
    let mut command = Command::new("explorer.exe");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn open_legacy_ui() -> Result<(), String> {
    let path = env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("Conduit.exe");
    if !path.is_file() {
        return Err("The original Windows Share Target host is not installed".into());
    }
    let mut command = Command::new(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
        .arg("--legacy-ui")
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn open_nova_ui() -> Result<(), String> {
    // The production package intentionally retains Nova as a runnable
    // rollback, even if the Rune framework encounters a platform regression.
    let path = env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("conduit-nova.exe");
    if !path.is_file() {
        return Err("Conduit Nova is not installed alongside Rune".into());
    }
    let mut command = Command::new(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command.spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_default_is_dark_without_overriding_user_choice() {
        assert_eq!(normalized_theme(""), "dark");
        assert_eq!(normalized_theme("invalid"), "dark");
        assert_eq!(normalized_theme("system\n"), "system");
        assert_eq!(normalized_theme("light"), "light");
        assert_eq!(normalized_theme("dark"), "dark");
    }

    #[test]
    fn portable_install_owns_its_daemon_and_never_infers_scoop_identity() {
        let dir = std::env::temp_dir().join(format!(
            "conduit-rune-portable-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let app = dir.join("conduit-rune.exe");
        let daemon = dir.join("conduit-daemon.exe");
        fs::write(&daemon, []).unwrap();
        assert_eq!(portable_daemon_for(&app), Some(daemon));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn config_keeps_unrelated_keys_and_comments() {
        let output = merge_config(
            "# retained\none=1\ntray_icon=false\nextra=hello",
            &[
                ("tray_icon", "true".into()),
                ("relay_proxy", "system".into()),
            ],
        );
        assert!(
            output.contains("# retained\none=1\ntray_icon=true\nextra=hello\nrelay_proxy=system\n")
        );
    }

    #[test]
    fn rejects_malformed_transfer_and_paths() {
        assert!(parse_transfer("x\toutbound\t0\t1\tqueued\tfile\tdetail").is_none());
        assert!(parse_transfer("aabb\toutbound\t0\t1\tqueued\tfile\tdetail").is_some());
        assert!(
            validate(&Settings {
                relays: vec!["bad;endpoint".into()],
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            validate(&Settings {
                proxy_mode: "invalid".into(),
                ..Default::default()
            })
            .is_err()
        );
    }
}
