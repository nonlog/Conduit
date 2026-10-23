use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fs::{self, File},
    io::{Read, Write},
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
    pub paired: bool,
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
    pub desktop_channel: String,
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

fn tray_icon_enabled(config: &BTreeMap<String, String>) -> bool {
    // Match the daemon's default (true) and its four accepted truthy spellings.
    config
        .get("tray_icon")
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "true" | "1" | "yes" | "on"
            )
        })
        .unwrap_or(true)
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

fn scoop_install_root_from_exe(path: &Path) -> Option<PathBuf> {
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
    Some(apps.parent()?.to_owned())
}

fn scoop_root_from_exe(path: &Path) -> Option<PathBuf> {
    scoop_install_root_from_exe(path).and_then(valid_scoop_root)
}

fn portable_stable_daemon(ui_exe: &Path) -> Option<PathBuf> {
    // A manually extracted stable Windows ZIP remains a portable installation:
    // both the daemon and its data/ directory live beside conduit-nova.exe.
    // Crucially, it must NOT quietly use an unrelated user's Scoop daemon or
    // paired identity just because Scoop happens to be installed as well.
    if !ui_exe
        .file_name()?
        .to_string_lossy()
        .eq_ignore_ascii_case("conduit-nova.exe")
        || scoop_install_root_from_exe(ui_exe).is_some()
    {
        return None;
    }
    let daemon = ui_exe.with_file_name("conduit-daemon.exe");
    daemon.is_file().then_some(daemon)
}

fn own_portable_daemon() -> Option<PathBuf> {
    env::current_exe()
        .ok()
        .as_deref()
        .and_then(portable_stable_daemon)
}

fn scoop_root() -> Option<PathBuf> {
    // Scoop's own current executable path is authoritative even when both
    // sign-in and Explorer integration are disabled on a custom Scoop root.
    if let Some(root) = env::current_exe()
        .ok()
        .as_deref()
        .and_then(scoop_root_from_exe)
    {
        return Some(root);
    }
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
    if let Some(daemon) = own_portable_daemon() {
        return daemon.parent().unwrap().join("data");
    }
    if let Some(scoop) = scoop_root() {
        let persisted = scoop.join("persist").join("conduit").join("data");
        // A verified Scoop installation owns this directory even before the
        // very first pairing. Returning LocalAppData until identity.bin exists
        // would hide freshly-created pairing and transfer status from the UI.
        return persisted;
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
    if let Some(daemon) = own_portable_daemon() {
        return Some(daemon);
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
        tray_icon: tray_icon_enabled(&config),
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
            paired: !paired_id.is_empty(),
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
        desktop_channel: if app.config().identifier.ends_with(".preview") {
            "preview".into()
        } else {
            "stable".into()
        },
        data_dir: dir.to_string_lossy().into_owned(),
        daemon_available: daemon_path(app).is_some(),
    }
}

fn merge_config(original: &str, changes: &[(&str, String)]) -> String {
    let replacements = changes
        .iter()
        .map(|(key, value)| (*key, value))
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
        // Do not silently destroy existing comments and unrelated settings.
        lines.push(line.to_owned());
    }
    for (key, value) in changes {
        if !written.contains(*key) {
            lines.push(format!("{key}={value}"));
        }
    }
    format!("{}\n", lines.join("\n"))
}

fn write_config_atomically(file: &Path, contents: &str) -> Result<(), String> {
    // Write to a new file in the same directory, then replace in one rename.
    // A daemon reload must never observe a partially truncated config.txt.
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = file.with_file_name(format!("config.{}.{}.tmp", std::process::id(), nonce));
    let result = (|| -> std::io::Result<()> {
        let mut writer = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        writer.write_all(contents.as_bytes())?;
        writer.sync_all()?;
        drop(writer);
        fs::rename(&temporary, file)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|e| format!("Saving settings: {e}"))
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
    let original = match fs::read_to_string(&file) {
        Ok(contents) if contents.len() <= 128 * 1024 => contents,
        Ok(_) => return Err("Existing configuration is too large to safely update".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("Reading existing settings: {e}")),
    };
    let updates = [
        ("relays", input.relays.join(";")),
        (
            "relay_proxy",
            match input.proxy_mode.as_str() {
                "system" => "system".into(),
                "manual" => input.manual_proxy.trim().into(),
                _ => String::new(),
            },
        ),
        ("tray_icon", input.tray_icon.to_string()),
        ("receive_dir", input.receive_dir.trim().into()),
    ];
    let contents = merge_config(&original, &updates);
    // Use the existing daemon reload contract; never overwrite identity/peer/history.
    write_config_atomically(&file, &contents)?;
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

pub fn open_legacy_ui() -> Result<(), String> {
    // A stable Scoop package keeps its original WinUI desktop alongside Nova
    // for Share Target activation and a deliberate user-accessible rollback.
    // Never accept a caller-supplied path or command-line argument.
    let candidate = env::current_exe()
        .map_err(|e| format!("Finding desktop installation: {e}"))?
        .with_file_name("Conduit.exe");
    if !candidate.is_file() || !candidate.with_file_name("conduit-nova.exe").is_file() {
        return Err("Legacy desktop is available only in a released Conduit installation".into());
    }
    let mut launch = Command::new(candidate);
    launch.arg("--legacy-ui");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        launch.creation_flags(0x08000000);
    }
    launch
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Opening legacy desktop: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_stable_bundle_keeps_its_own_daemon_and_data_root() {
        let root = env::temp_dir().join(format!(
            "conduit-nova-portable-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let nova = root.join("conduit-nova.exe");
        assert!(portable_stable_daemon(&nova).is_none());
        let daemon = root.join("conduit-daemon.exe");
        fs::write(&daemon, []).unwrap();
        assert_eq!(portable_stable_daemon(&nova), Some(daemon));
        assert!(portable_stable_daemon(&root.join("conduit-tauri-preview.exe")).is_none());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn paired_state_never_serializes_the_private_peer_identifier() {
        let view = Pairing {
            paired: true,
            device_name: "Phone".into(),
            code: "".into(),
            expires_ms: 0,
            active: false,
        };
        let json = serde_json::to_value(view).unwrap();
        assert_eq!(json["paired"], true);
        assert!(json.get("deviceId").is_none());
        assert!(json.get("device_id").is_none());
    }
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
    #[test]
    fn preserves_daemon_tray_default_and_aliases() {
        let mut config = BTreeMap::new();
        assert!(tray_icon_enabled(&config));
        config.insert("tray_icon".into(), "on".into());
        assert!(tray_icon_enabled(&config));
        config.insert("tray_icon".into(), "false".into());
        assert!(!tray_icon_enabled(&config));
    }
    #[test]
    fn settings_merge_keeps_comments_and_other_fields() {
        let changes = [
            ("relays", "conduit.example:41113".to_owned()),
            ("tray_icon", "true".to_owned()),
        ];
        let merged = merge_config(
            "# keep this note\nrelays=old:123\nother=untouched\nrelays=duplicate:123\n",
            &changes,
        );
        assert_eq!(
            merged,
            "# keep this note\nrelays=conduit.example:41113\nother=untouched\ntray_icon=true\n"
        );
    }
    #[test]
    fn atomic_save_replaces_content_without_leftover_temp_file() {
        let dir = std::env::temp_dir().join(format!(
            "conduit-tauri-config-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("config.txt");
        fs::write(&path, "old=true\n").unwrap();
        write_config_atomically(&path, "tray_icon=true\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "tray_icon=true\n");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }
}
