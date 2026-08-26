#![windows_subsystem = "windows"]

//! Small, on-demand Windows control surface.
//!
//! It owns no transport, tray icon, worker thread or timer. Opening the window reads the daemon's
//! event-written `status.txt` and `config.txt`; Refresh does the same on demand. Closing the window
//! ends the process completely.

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::ptr::{null, null_mut};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::SetWindowTheme;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const ID_REFRESH: usize = 101;
const ID_SAVE: usize = 102;
const ID_FOLDER: usize = 103;
const ID_AUTOSTART: usize = 104;
const ID_EXPLORER: usize = 105;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Default)]
struct Ui {
    window: isize,
    status: isize,
    relays: isize,
    proxy: isize,
    autostart: isize,
    explorer: isize,
    config_dir: PathBuf,
    daemon: PathBuf,
}

static UI: OnceLock<Mutex<Ui>> = OnceLock::new();

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn config_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("Conduit")
}

fn sibling(name: &str) -> PathBuf {
    std::env::current_exe()
        .unwrap_or_default()
        .with_file_name(name)
}

unsafe fn set_text(hwnd: isize, text: &str) {
    let text = wide(text);
    SetWindowTextW(hwnd as HWND, text.as_ptr());
}

unsafe fn get_text(hwnd: isize) -> String {
    let len = GetWindowTextLengthW(hwnd as HWND);
    let mut buffer = vec![0u16; len as usize + 1];
    let got = GetWindowTextW(hwnd as HWND, buffer.as_mut_ptr(), buffer.len() as i32);
    String::from_utf16_lossy(&buffer[..got as usize])
}

fn read_pairs(path: PathBuf) -> Vec<(String, String)> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

fn value(pairs: &[(String, String)], key: &str) -> String {
    pairs
        .iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value.clone())
        .unwrap_or_default()
}

fn registry_value(key: &str, name: &str) -> bool {
    windows_registry::CURRENT_USER
        .open(key)
        .ok()
        .and_then(|key| key.get_string(name).ok())
        .is_some()
}

unsafe fn refresh() {
    let Some(ui) = UI.get() else { return };
    let ui = ui.lock().expect("ui mutex poisoned");
    let status = read_pairs(ui.config_dir.join("status.txt"));
    let config = read_pairs(ui.config_dir.join("config.txt"));

    let daemon = if std::net::TcpListener::bind(("0.0.0.0", 41112)).is_err() {
        "Running"
    } else {
        "Stopped"
    };
    let state = value(&status, "state");
    let peer = value(&status, "peer_name");
    let path = value(&status, "path");
    let relay = value(&status, "relay");
    let summary = format!(
        "Desktop daemon: {daemon}\r\nLink: {}\r\nPhone: {}\r\nPath: {}{}",
        if state.is_empty() { "unknown" } else { &state },
        if peer.is_empty() { "—" } else { &peer },
        if path.is_empty() { "—" } else { &path },
        if relay.is_empty() { String::new() } else { format!(" · {relay}") },
    );
    set_text(ui.status, &summary);
    set_text(ui.relays, &value(&config, "relays"));
    set_text(ui.proxy, &value(&config, "relay_proxy"));

    SendMessageW(
        ui.autostart as HWND,
        BM_SETCHECK,
        if registry_value(r"Software\Microsoft\Windows\CurrentVersion\Run", "Conduit") {
            1
        } else {
            0
        },
        0,
    );
    SendMessageW(
        ui.explorer as HWND,
        BM_SETCHECK,
        if windows_registry::CURRENT_USER
            .open(r"Software\Classes\*\shell\Conduit.SendToPhone")
            .is_ok()
        {
            1
        } else {
            0
        },
        0,
    );
}

unsafe fn save_config() {
    let Some(ui) = UI.get() else { return };
    let ui = ui.lock().expect("ui mutex poisoned");
    let relays = get_text(ui.relays).replace(['\r', '\n'], "");
    let proxy = get_text(ui.proxy).replace(['\r', '\n'], "");
    if fs::create_dir_all(&ui.config_dir).is_err()
        || fs::write(
            ui.config_dir.join("config.txt"),
            format!("relays={relays}\nrelay_proxy={proxy}\n"),
        )
        .is_err()
    {
        message("Could not save Conduit settings.", MB_ICONERROR);
        return;
    }
    message(
        "Settings saved. Restart the desktop daemon to apply Relay changes.",
        MB_ICONINFORMATION,
    );
}

fn run_daemon_command(args: &[&str]) -> bool {
    let daemon = UI
        .get()
        .and_then(|ui| ui.lock().ok().map(|ui| ui.daemon.clone()))
        .unwrap_or_else(|| sibling("conduit-daemon.exe"));
    Command::new(daemon)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

unsafe fn toggle(kind: usize) {
    let Some(ui) = UI.get() else { return };
    let ui = ui.lock().expect("ui mutex poisoned");
    let hwnd = if kind == ID_AUTOSTART { ui.autostart } else { ui.explorer };
    let checked = SendMessageW(hwnd as HWND, BM_GETCHECK, 0, 0) == 1;
    drop(ui);
    let target = if kind == ID_AUTOSTART { "autostart" } else { "explorer" };
    if !run_daemon_command(&[target, if checked { "install" } else { "remove" }]) {
        message("Could not update the Windows integration.", MB_ICONERROR);
        refresh();
    }
}

unsafe fn message(text: &str, icon: MESSAGEBOX_STYLE) {
    let title = wide("Conduit");
    let text = wide(text);
    let parent = UI
        .get()
        .and_then(|ui| ui.lock().ok().map(|ui| ui.window as HWND))
        .unwrap_or(null_mut());
    MessageBoxW(parent, text.as_ptr(), title.as_ptr(), MB_OK | icon);
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            SetBkMode(wparam as HDC, TRANSPARENT as i32);
            GetStockObject(WHITE_BRUSH) as LRESULT
        }
        WM_COMMAND => {
            let id = wparam & 0xffff;
            match id {
                ID_REFRESH => refresh(),
                ID_SAVE => save_config(),
                ID_FOLDER => {
                    if let Some(dir) = UI.get().and_then(|ui| ui.lock().ok().map(|ui| ui.config_dir.clone())) {
                        let _ = Command::new("explorer.exe").arg(dir).spawn();
                    }
                }
                ID_AUTOSTART | ID_EXPLORER => toggle(id),
                _ => {}
            }
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn child(
    class: &str,
    text: &str,
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    parent: HWND,
    id: usize,
    instance: HINSTANCE,
) -> HWND {
    let class = wide(class);
    let text = wide(text);
    CreateWindowExW(
        0,
        class.as_ptr(),
        text.as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        x,
        y,
        w,
        h,
        parent,
        id as HMENU,
        instance,
        null(),
    )
}

fn main() {
    unsafe {
        let instance = GetModuleHandleW(null());
        let class_name = wide("ConduitControlWindow");
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(wnd_proc);
        wc.hInstance = instance;
        wc.lpszClassName = class_name.as_ptr();
        wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        wc.hbrBackground = GetStockObject(WHITE_BRUSH) as HBRUSH;
        if RegisterClassW(&wc) == 0 {
            return;
        }

        let title = wide("Conduit");
        let window = CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            560,
            430,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if window.is_null() {
            return;
        }

        let font = GetStockObject(DEFAULT_GUI_FONT);
        let label = |text: &str, x, y, w, h| {
            let hwnd = child("STATIC", text, 0, x, y, w, h, window, 0, instance);
            SendMessageW(hwnd, WM_SETFONT, font as usize, 1);
            hwnd
        };
        label("Connection", 24, 20, 160, 22);
        let status = child("STATIC", "", 0, 24, 48, 500, 82, window, 0, instance);
        label("Relay endpoints", 24, 142, 180, 22);
        let relays = child(
            "EDIT",
            "",
            WS_BORDER | ES_AUTOHSCROLL as u32,
            24,
            166,
            500,
            26,
            window,
            0,
            instance,
        );
        label("Relay SOCKS5 proxy (blank = direct)", 24, 207, 300, 22);
        let proxy = child(
            "EDIT",
            "",
            WS_BORDER | ES_AUTOHSCROLL as u32,
            24,
            231,
            500,
            26,
            window,
            0,
            instance,
        );
        let autostart = child(
            "BUTTON",
            "Start Conduit when I sign in",
            BS_AUTOCHECKBOX as u32,
            24,
            275,
            240,
            24,
            window,
            ID_AUTOSTART,
            instance,
        );
        let explorer = child(
            "BUTTON",
            "Show Send to phone in Explorer",
            BS_AUTOCHECKBOX as u32,
            280,
            275,
            245,
            24,
            window,
            ID_EXPLORER,
            instance,
        );
        let refresh_button = child("BUTTON", "Refresh", (BS_PUSHBUTTON | BS_FLAT) as u32, 24, 325, 100, 30, window, ID_REFRESH, instance);
        let save = child("BUTTON", "Save settings", (BS_DEFPUSHBUTTON | BS_FLAT) as u32, 136, 325, 120, 30, window, ID_SAVE, instance);
        let folder = child("BUTTON", "Open diagnostics", (BS_PUSHBUTTON | BS_FLAT) as u32, 268, 325, 130, 30, window, ID_FOLDER, instance);

        for hwnd in [status, relays, proxy, autostart, explorer, refresh_button, save, folder] {
            SendMessageW(hwnd, WM_SETFONT, font as usize, 1);
            let explorer_theme = wide("Explorer");
            SetWindowTheme(hwnd, explorer_theme.as_ptr(), null());
        }

        UI.set(Mutex::new(Ui {
            window: window as isize,
            status: status as isize,
            relays: relays as isize,
            proxy: proxy as isize,
            autostart: autostart as isize,
            explorer: explorer as isize,
            config_dir: config_dir(),
            daemon: sibling("conduit-daemon.exe"),
        }))
        .ok();

        refresh();
        ShowWindow(window, SW_SHOW);
        UpdateWindow(window);

        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}
