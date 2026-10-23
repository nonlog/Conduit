use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    env,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub state: String,
    pub route: String,
    pub relay: String,
    pub device_name: String,
    pub linked: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pairing {
    pub device_id: String,
    pub device_name: String,
    pub code: String,
    pub expires_ms: u64,
    pub active: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub relays: Vec<String>,
    pub proxy_mode: String,
    pub manual_proxy: String,
    pub receive_dir: String,
    pub tray_icon: bool,
    pub autostart: bool,
    pub explorer_integration: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    pub id: String,
    pub app: String,
    pub title: String,
    pub body: String,
    pub timestamp: u64,
    pub icon: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedLink {
    pub id: String,
    pub url: String,
    pub title: String,
    pub source: String,
    pub timestamp: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub id: String,
    pub name: String,
    pub direction: String,
    pub bytes: u64,
    pub total: u64,
    pub state: String,
    pub detail: String,
    pub percentage: f64,
    pub terminal: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub connection: Connection,
    pub pairing: Pairing,
    pub settings: Settings,
    pub notifications: Vec<Notification>,
    pub shared_links: Vec<SharedLink>,
    pub transfers: Vec<Transfer>,
    pub wallpaper: Option<String>,
    pub version: &'static str,
    pub data_dir: String,
    pub daemon_available: bool,
}

fn read_bounded(path: &Path, cap: u64) -> String {
    let Ok(file) = File::open(path) else {
        return String::new();
    };
    let mut text = String::new();
    if file.take(cap).read_to_string(&mut text).is_ok() {
        text
    } else {
        String::new()
    }
}

fn kv(path: &Path) -> BTreeMap<String, String> {
    read_bounded(path, 128 * 1024)
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect()
}

fn get(values: &BTreeMap<String, String>, key: &str) -> String {
    values.get(key).cloned().unwrap_or_default()
}

fn valid_scoop_root(root: PathBuf) -> Option<PathBuf> {
    // Never treat a guessed Scoop directory as authoritative unless it really
    // contains the installed Conduit daemon. This also prevents false identity
    // resets when users install Scoop in a custom location.
    root.join("apps")
        .join("conduit")
        .join("current")
        .join("conduit-daemon.exe")
        .is_file()
        .then_some(root)
}

fn scoop_root_from_exe(path: &Path) -> Option<PathBuf> {
    let current = path.parent()?;
    let conduit = current.parent()?;
    let apps = conduit.parent()?;
    if !conduit
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
    valid_scoop_root(apps.parent()?.to_owned())
}

fn scoop_root() -> Option<PathBuf> {
    // A disabled startup entry MUST NOT hide existing pairing and history.
    #[cfg(windows)]
    {
        use winreg::{enums::HKEY_CURRENT_USER, RegKey};
        let root = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(run) = root.open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run") {
            if let Ok(command) = run.get_value::<String, _>("Conduit") {
                let path = PathBuf::from(command.trim().trim_matches('"'));
                if let Some(result) = scoop_root_from_exe(&path) {
                    return Some(result);
                }
            }
        }
    }
    if let Some(root) = env::var_os("SCOOP")
        .map(PathBuf::from)
        .and_then(valid_scoop_root)
    {
        return Some(root);
    }
    // Scoop's shims directory is on PATH even if users turn off Conduit
    // autostart or Explorer integration. This supports nonstandard Scoop roots.
    if let Some(paths) = env::var_os("PATH") {
        for shim_dir in env::split_paths(&paths) {
            if shim_dir
                .file_name()
                .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("shims"))
            {
                if let Some(root) = shim_dir
                    .parent()
                    .map(Path::to_path_buf)
                    .and_then(valid_scoop_root)
                {
                    return Some(root);
                }
            }
        }
    }
    env::var_os("USERPROFILE")
        .map(|user| PathBuf::from(user).join("scoop"))
        .and_then(valid_scoop_root)
}

pub fn data_dir() -> PathBuf {
    if let Some(override_path) = env::var_os("CONDUIT_DATA_DIR") {
        if !override_path.is_empty() {
            return PathBuf::from(override_path);
        }
    }
    if let Some(scoop) = scoop_root() {
        let persisted = scoop.join("persist").join("conduit").join("data");
        if persisted.join("identity.bin").is_file() || persisted.join("peer.txt").is_file() {
            return persisted;
        }
    }
    // The native daemon also uses this fallback outside Scoop. Never migrate or
    // synthesize an identity from the UI, and never store pairing data in WebView.
    let base = env::var_os("LOCALAPPDATA")
        .or_else(|| env::var_os("APPDATA"))
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir);
    base.join("Conduit")
}

pub fn daemon_path(app: &AppHandle) -> Option<PathBuf> {
    if let Some(path) = env::var_os("CONDUIT_DAEMON_PATH") {
        let candidate = PathBuf::from(path);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    // Prefer the installed daemon, already owning the Noise session and autostart.
    if let Some(root) = scoop_root() {
        let installed = root
            .join("apps")
            .join("conduit")
            .join("current")
            .join("conduit-daemon.exe");
        if installed.is_file() {
            return Some(installed);
        }
    }
    // CI packages the matching version only for standalone preview installations.
    if let Ok(dir) = app.path().resource_dir() {
        let bundled = dir.join("resources").join("conduit-daemon.exe");
        if bundled.is_file() {
            return Some(bundled);
        }
    }
    None
}

fn data_url(path: &Path, mime: &str, max_bytes: u64) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() == 0 || metadata.len() > max_bytes {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

pub fn parse_transfer(line: &str) -> Option<Transfer> {
    let parts: Vec<&str> = line.splitn(7, '\t').collect();
    if parts.len() != 7 {
        return None;
    }
    let id = parts[0];
    if !(2..=128).contains(&id.len())
        || id.len() % 2 != 0
        || !id.bytes().all(|c| c.is_ascii_hexdigit())
        || !["inbound", "outbound"].contains(&parts[1])
        || ![
            "queued",
            "transferring",
            "waiting confirmation",
            "completed",
            "failed",
            "cancelled",
        ]
        .contains(&parts[4])
    {
        return None;
    }
    let bytes = parts[2].parse::<u64>().ok()?;
    let total = parts[3].parse::<u64>().ok()?;
    if bytes > total {
        return None;
    }
    let terminal = ["completed", "failed", "cancelled"].contains(&parts[4]);
    Some(Transfer {
        id: id.to_owned(),
        name: parts[5].to_owned(),
        direction: parts[1].to_owned(),
        bytes,
        total,
        state: parts[4].to_owned(),
        detail: parts[6].to_owned(),
        percentage: if total == 0 {
            0.0
        } else {
            100.0 * bytes as f64 / total as f64
        },
        terminal,
    })
}

pub fn snapshot(app: &AppHandle) -> Snapshot {
    let dir = data_dir();
    let status = kv(&dir.join("status.txt"));
    let config = kv(&dir.join("config.txt"));
    let pair = kv(&dir.join("pairing.txt"));
    let paired_id = read_bounded(&dir.join("peer.txt"), 512).trim().to_owned();
    let paired_name = read_bounded(&dir.join("peer-name.txt"), 1024)
        .trim()
        .to_owned();
    let expire = get(&pair, "expires_ms").parse::<u64>().unwrap_or(0);
    let code = get(&pair, "code");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let pairing_active =
        expire > now && code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit());
    let raw_state = get(&status, "state");
    let connection = Connection {
        linked: raw_state == "linked",
        state: match raw_state.as_str() {
            "linked" => "Connected",
            "connecting" => "Connecting",
            "retrying" => "Reconnecting",
            "" => "Disconnected",
            _ => "Disconnected",
        }
        .into(),
        device_name: {
            let current = get(&status, "peer_name");
            if !current.is_empty() {
                current
            } else if !paired_name.is_empty() {
                paired_name.clone()
            } else {
                "No phone linked".into()
            }
        },
        route: get(&status, "path"),
        relay: get(&status, "relay"),
    };
    let relay_proxy = get(&config, "relay_proxy");
    let (proxy_mode, manual_proxy) = match relay_proxy.as_str() {
        "system" => ("system", ""),
        "" => ("direct", ""),
        _ => ("manual", relay_proxy.as_str()),
    };
    #[cfg(windows)]
    let (autostart, explorer_integration) = {
        use winreg::{enums::HKEY_CURRENT_USER, RegKey};
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let auto = root
            .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
            .and_then(|key| key.get_value::<String, _>("Conduit"))
            .is_ok();
        let explorer = root
            .open_subkey("Software\\Classes\\*\\shell\\Conduit.SendToPhone")
            .is_ok();
        (auto, explorer)
    };
    #[cfg(not(windows))]
    let (autostart, explorer_integration) = (false, false);

    let settings = Settings {
        relays: get(&config, "relays")
            .split([';', '\r', '\n', ','])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect(),
        proxy_mode: proxy_mode.into(),
        manual_proxy: manual_proxy.into(),
        receive_dir: get(&config, "receive_dir"),
        tray_icon: get(&config, "tray_icon").eq_ignore_ascii_case("true"),
        autostart,
        explorer_integration,
    };
    let notifications = read_bounded(&dir.join("notifications.tsv"), 2 * 1024 * 1024)
        .lines()
        .take(100)
        .enumerate()
        .filter_map(|(i, line)| {
            let fields: Vec<_> = line.splitn(6, '\t').collect();
            if fields.len() < 2 {
                return None;
            }
            let timestamp = fields[0].parse::<u64>().ok()?;
            let package = fields.get(2).copied().unwrap_or("");
            let app_name = fields
                .get(3)
                .copied()
                .filter(|s| !s.is_empty())
                .unwrap_or(package);
            let title = fields
                .get(4)
                .copied()
                .filter(|s| !s.is_empty())
                .unwrap_or("Notification");
            // Match the existing WinUI icon filename protocol exactly. Keep the
            // icon bytes in a bounded data URI; WebView never sees a filesystem path.
            let icon = if package.is_empty() {
                None
            } else {
                let hash = Sha256::digest(package.as_bytes());
                let filename = hash[..8]
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                data_url(
                    &dir.join("icons").join(format!("{filename}.png")),
                    "image/png",
                    64 * 1024,
                )
            };
            Some(Notification {
                id: format!("{timestamp}-{i}"),
                app: if app_name.is_empty() {
                    "Phone".into()
                } else {
                    app_name.into()
                },
                title: title.into(),
                body: fields.get(5).copied().unwrap_or("").into(),
                timestamp,
                icon,
            })
        })
        .collect();
    let shared_links = read_bounded(&dir.join("shared-links.tsv"), 2 * 1024 * 1024)
        .lines()
        .take(200)
        .enumerate()
        .filter_map(|(i, line)| {
            let fields: Vec<_> = line.split('\t').collect();
            let timestamp = fields.first()?.parse::<u64>().ok()?;
            let url = fields.get(1)?.trim();
            let parsed = url::Url::parse(url).ok()?;
            if !["http", "https"].contains(&parsed.scheme()) {
                return None;
            }
            Some(SharedLink {
                id: format!("{timestamp}-{i}"),
                url: url.into(),
                title: fields
                    .get(2)
                    .copied()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(url)
                    .into(),
                source: fields
                    .get(3)
                    .copied()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Phone")
                    .into(),
                timestamp,
            })
        })
        .collect();
    let transfers = read_bounded(&dir.join("transfers.tsv"), 2 * 1024 * 1024)
        .lines()
        .take(32)
        .filter_map(parse_transfer)
        .collect();
    Snapshot {
        connection,
        pairing: Pairing {
            device_id: paired_id,
            device_name: paired_name,
            code: if pairing_active { code } else { String::new() },
            expires_ms: expire,
            active: pairing_active,
        },
        settings,
        notifications,
        shared_links,
        transfers,
        wallpaper: data_url(&dir.join("wallpaper.jpg"), "image/jpeg", 1024 * 1024),
        version: env!("CARGO_PKG_VERSION"),
        data_dir: dir.to_string_lossy().into_owned(),
        daemon_available: daemon_path(app).is_some(),
    }
}

pub fn save_settings(input: Settings, app: &AppHandle) -> Result<(), String> {
    if input.relays.len() > 12
        || !["system", "direct", "manual"].contains(&input.proxy_mode.as_str())
    {
        return Err("Invalid relay or proxy configuration".into());
    }
    if input.relays.iter().any(|endpoint| {
        endpoint.len() > 255
            || !endpoint.is_ascii()
            || endpoint
                .chars()
                .any(|c| !(c.is_ascii_alphanumeric() || ".:-_".contains(c)))
    }) {
        return Err("Relay entries must be host:port without spaces or special characters".into());
    }
    if input.manual_proxy.len() > 512
        || input.manual_proxy.chars().any(char::is_control)
        || input.receive_dir.len() > 1024
        || input.receive_dir.chars().any(char::is_control)
    {
        return Err("Invalid proxy or receive folder".into());
    }
    let dir = data_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("Creating settings directory: {e}"))?;
    let file = dir.join("config.txt");
    let mut existing = kv(&file);
    existing.insert("relays".into(), input.relays.join(";"));
    existing.insert(
        "relay_proxy".into(),
        match input.proxy_mode.as_str() {
            "system" => "system".into(),
            "manual" => input.manual_proxy.trim().into(),
            _ => String::new(),
        },
    );
    existing.insert("tray_icon".into(), input.tray_icon.to_string());
    existing.insert("receive_dir".into(), input.receive_dir.trim().into());
    let contents = existing
        .iter()
        .map(|(k, v)| format!("{k}={v}\n"))
        .collect::<String>();
    // Use the existing daemon reload contract; never overwrite identity/peer/history.
    fs::write(&file, contents).map_err(|e| format!("Saving settings: {e}"))?;
    let _ = daemon_command(app, &["reload"])?;
    Ok(())
}

fn command(app: &AppHandle) -> Result<Command, String> {
    let exe = daemon_path(app).ok_or("No Conduit daemon was found. Keep the existing Windows installation or use the preview bundle.")?;
    let mut process = Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        process.creation_flags(0x08000000);
    }
    Ok(process)
}

pub fn daemon_command(app: &AppHandle, arguments: &[&str]) -> Result<String, String> {
    let mut process = command(app)?;
    let result = process
        .args(arguments)
        .output()
        .map_err(|e| format!("Starting daemon command: {e}"))?;
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    if !result.status.success() {
        let reason = stderr.trim().trim_start_matches("Error: ");
        return Err(if reason.is_empty() {
            format!("Command failed: {}", result.status)
        } else {
            reason.to_owned()
        });
    }
    Ok(stdout.trim().to_owned())
}

pub fn send_file(app: &AppHandle, path: &str) -> Result<(), String> {
    let path = PathBuf::from(path);
    let metadata = fs::metadata(&path).map_err(|e| format!("Selected file is unavailable: {e}"))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err("Select an existing, nonempty file".into());
    }
    let path = fs::canonicalize(path).map_err(|e| format!("Resolving file path: {e}"))?;
    let mut process = command(app)?;
    // The daemon streams progress until the peer acknowledges the transfer.
    // Do not block Tauri's UI thread or kill a queued transfer if its window closes.
    process
        .args(["send"])
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = process.spawn().map_err(|e| format!("Queueing file: {e}"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

pub fn launch_daemon_if_missing(app: &AppHandle) -> Result<(), String> {
    let running = Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq conduit-daemon.exe", "/NH"])
        .output()
        .map_err(|e| format!("Checking daemon: {e}"))?;
    if String::from_utf8_lossy(&running.stdout)
        .to_lowercase()
        .contains("conduit-daemon.exe")
    {
        return Ok(());
    }
    command(app)?
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Starting Conduit: {e}"))
}

pub fn clear_history(kind: &str) -> Result<(), String> {
    let file = match kind {
        "notifications" => "notifications.tsv",
        "links" => "shared-links.tsv",
        _ => return Err("Only notification or shared link history can be cleared".into()),
    };
    let path = data_dir().join(file);
    if path.is_file() {
        fs::remove_file(path).map_err(|e| format!("Clearing history: {e}"))?;
    }
    Ok(())
}

pub fn open_external(url: &str) -> Result<(), String> {
    if url.len() > 2048 {
        return Err("Link is too long".into());
    }
    let parsed = url::Url::parse(url).map_err(|_| "Invalid link")?;
    if !["https", "http"].contains(&parsed.scheme()) {
        return Err("Only http and https links are supported".into());
    }
    let mut process = Command::new("explorer.exe");
    process.arg(url);
    process
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Opening link: {e}"))
}

pub fn open_directory(path: &Path) -> Result<(), String> {
    if !path.is_dir() {
        return Err("Folder does not exist".into());
    }
    Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Opening folder: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_real_transfer_records() {
        let row = "aabbccdd\tinbound\t2097152\t2097152\tcompleted\tfile.mp3\t";
        let record = parse_transfer(row).unwrap();
        assert!(record.terminal);
        assert_eq!(record.percentage, 100.0);
        assert_eq!(record.name, "file.mp3");
        assert!(parse_transfer("xx\toutbound\t1\t1\tcompleted\tx\t").is_none());
        assert!(parse_transfer("aabb\toutbound\t999\t1\tqueued\tx\t").is_none());
    }
    #[test]
    fn rejects_incomplete_or_invalid_transfer_states() {
        assert!(parse_transfer("aabb\toutbound\t1\t2\tcancelled\tfile\tCancelled").is_some());
        assert!(parse_transfer("aabb\toutbound\t1\t2\texecuting\tfile\t").is_none());
        assert!(parse_transfer("aabb\toutbound\t1\t2\tqueued\tfile").is_none());
    }
}
