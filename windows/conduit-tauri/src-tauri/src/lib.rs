mod bridge;

use notify::{RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter};

#[tauri::command]
fn conduit_snapshot(app: AppHandle) -> bridge::Snapshot {
    bridge::snapshot(&app)
}

#[tauri::command]
async fn ensure_daemon(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || bridge::launch_daemon_if_missing(&app))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn daemon_action(
    app: AppHandle,
    action: String,
    transfer_id: Option<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || match action.as_str() {
        "pair-start" => bridge::daemon_command(&app, &["pair", "start"]),
        "pair-cancel" => bridge::daemon_command(&app, &["pair", "cancel"]),
        "pair-forget" => bridge::daemon_command(&app, &["pair", "forget"]),
        "autostart-enable" => bridge::daemon_command(&app, &["autostart", "install"]),
        "autostart-disable" => bridge::daemon_command(&app, &["autostart", "remove"]),
        "explorer-enable" => bridge::daemon_command(&app, &["explorer", "install"]),
        "explorer-disable" => bridge::daemon_command(&app, &["explorer", "remove"]),
        "reload" => bridge::daemon_command(&app, &["reload"]),
        "cancel-transfer" => {
            let id = transfer_id.ok_or("Missing transfer ID")?;
            if !(2..=128).contains(&id.len())
                || id.len() % 2 != 0
                || !id.bytes().all(|c| c.is_ascii_hexdigit())
            {
                return Err("Invalid transfer ID".into());
            }
            bridge::daemon_command(&app, &["cancel-transfer", &id])
        }
        _ => Err("Unrecognized daemon action".into()),
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn send_file(app: AppHandle, path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || bridge::send_file(&app, &path))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn save_settings(app: AppHandle, settings: bridge::Settings) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || bridge::save_settings(settings, &app))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn clear_history(kind: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || bridge::clear_history(&kind))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn open_link(url: String) -> Result<(), String> {
    bridge::open_external(&url)
}

#[tauri::command]
fn open_folder(kind: String, app: AppHandle) -> Result<(), String> {
    let path = match kind.as_str() {
        "data" => bridge::data_dir(),
        "received" => {
            let path = bridge::snapshot(&app).settings.receive_dir;
            if path.is_empty() {
                std::env::var_os("USERPROFILE")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_default()
                    .join("Downloads")
            } else {
                std::path::PathBuf::from(path)
            }
        }
        _ => return Err("Unknown folder".into()),
    };
    bridge::open_directory(&path)
}

fn start_file_observer(app: AppHandle) {
    std::thread::spawn(move || {
        let folder = bridge::data_dir();
        if std::fs::create_dir_all(&folder).is_err() {
            return;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(move |event| {
            let _ = sender.send(event);
        }) {
            Ok(watcher) => watcher,
            Err(_) => return,
        };
        if watcher.watch(&folder, RecursiveMode::NonRecursive).is_err() {
            return;
        }
        while let Ok(Ok(event)) = receiver.recv() {
            let relevant = event.paths.iter().any(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| {
                        matches!(
                            name.to_ascii_lowercase().as_str(),
                            "status.txt"
                                | "config.txt"
                                | "pairing.txt"
                                | "peer.txt"
                                | "peer-name.txt"
                                | "notifications.tsv"
                                | "shared-links.tsv"
                                | "transfers.tsv"
                                | "wallpaper.jpg"
                        )
                    })
                    .unwrap_or(false)
            });
            if relevant {
                let _ = app.emit("conduit:data-changed", ());
            }
        }
    });
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle().clone();
            start_file_observer(handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            conduit_snapshot,
            ensure_daemon,
            daemon_action,
            send_file,
            save_settings,
            clear_history,
            open_link,
            open_folder
        ])
        .run(tauri::generate_context!())
        .expect("Conduit desktop preview failed to start");
}
