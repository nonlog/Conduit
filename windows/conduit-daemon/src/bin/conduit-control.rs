#![windows_subsystem = "windows"]

//! Small, on-demand Windows control surface.
//!
//! It owns no transport, tray icon, worker thread or timer. Opening the window reads the daemon's
//! event-written `status.txt` and `config.txt`; Refresh does the same on demand. Closing the window
//! ends the process completely.

use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr::{null, null_mut};
use std::sync::OnceLock;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_ROUND,
};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::ApplicationInstallationAndServicing::{
    ActivateActCtx, CreateActCtxW, DeactivateActCtx, ReleaseActCtx, ACTCTXW,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::SetWindowTheme;
use windows_sys::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, GetDpiForWindow, SetProcessDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const ID_REFRESH: usize = 101;
const ID_SAVE: usize = 102;
const ID_FOLDER: usize = 103;
const ID_AUTOSTART: usize = 104;
const ID_EXPLORER: usize = 105;
const ID_TRAY: usize = 106;
const ID_TITLE: usize = 201;
const ID_CONNECTION_CAPTION: usize = 203;
const ID_CONNECTION_STATE: usize = 204;
const ID_CONNECTION_DETAIL: usize = 205;
const ID_ROUTING_CAPTION: usize = 206;
const ID_RELAY_LABEL: usize = 207;
const ID_PROXY_LABEL: usize = 208;
const ID_INTEGRATIONS_CAPTION: usize = 210;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const STATIC_NO_PREFIX: u32 = 0x0080;
const DEFAULT_RELAYS: &str = "us.414222.xyz:41113;tyo.414222.xyz:41113;wa.414222.xyz:41113";

#[derive(Clone, Copy)]
struct Theme {
    dark: bool,
    bg: COLORREF,
    card: COLORREF,
    border: COLORREF,
    edit: COLORREF,
    text: COLORREF,
    muted: COLORREF,
}

struct Ui {
    window: isize,
    status_state: isize,
    status_detail: isize,
    relays: isize,
    proxy: isize,
    autostart: isize,
    explorer: isize,
    tray: isize,
    config_dir: PathBuf,
    daemon: PathBuf,
    dpi: u32,
    theme: Theme,
    bg_brush: isize,
    card_brush: isize,
    edit_brush: isize,
    border_pen: isize,
    app_mark_connection: isize,
    app_mark_title: isize,
}

static UI: OnceLock<Ui> = OnceLock::new();

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | ((g as u32) << 8) | ((b as u32) << 16)
}

fn app_theme() -> Theme {
    let light = windows_registry::CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .ok()
        .and_then(|key| key.get_u32("AppsUseLightTheme").ok())
        .unwrap_or(1)
        != 0;
    if light {
        Theme {
            dark: false,
            bg: rgb(243, 243, 243),
            card: rgb(255, 255, 255),
            border: rgb(220, 224, 230),
            edit: rgb(255, 255, 255),
            text: rgb(27, 27, 27),
            muted: rgb(96, 96, 96),
        }
    } else {
        Theme {
            dark: true,
            bg: rgb(32, 32, 32),
            card: rgb(44, 44, 44),
            border: rgb(60, 60, 60),
            edit: rgb(50, 50, 50),
            text: rgb(245, 245, 245),
            muted: rgb(180, 180, 180),
        }
    }
}

fn dip(value: i32, dpi: u32) -> i32 {
    ((value as i64 * dpi as i64 + 48) / 96) as i32
}

unsafe fn font(dpi: u32, size: i32, weight: i32) -> HFONT {
    let face = wide("Segoe UI Variable Text");
    CreateFontW(
        -dip(size, dpi),
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32,
        CLIP_DEFAULT_PRECIS as u32,
        CLEARTYPE_QUALITY as u32,
        (DEFAULT_PITCH | FF_DONTCARE) as u32,
        face.as_ptr(),
    )
}

unsafe fn set_font(hwnd: HWND, font: HFONT) {
    SendMessageW(hwnd, WM_SETFONT, font as usize, 1);
}

unsafe fn set_native_theme(hwnd: HWND, dark: bool) {
    let name = wide(if dark {
        "DarkMode_Explorer"
    } else {
        "Explorer"
    });
    SetWindowTheme(hwnd, name.as_ptr(), null());
}

const APP_ICON_ICO: &[u8] = include_bytes!("../../assets/conduit-icon.ico");

fn ensure_app_icon_file() -> Option<PathBuf> {
    let dir = config_dir();
    if fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let path = dir.join("conduit-icon.ico");
    let current_matches = fs::read(&path)
        .map(|current| current.as_slice() == APP_ICON_ICO)
        .unwrap_or(false);
    if !current_matches && fs::write(&path, APP_ICON_ICO).is_err() {
        return None;
    }
    Some(path)
}

unsafe fn load_app_icon(path: &Path, size: i32) -> HICON {
    let wide_path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    LoadImageW(
        null_mut(),
        wide_path.as_ptr(),
        IMAGE_ICON,
        size,
        size,
        LR_LOADFROMFILE,
    ) as HICON
}

/// Activates Common Controls v6 without requiring `mt.exe` or a Visual Studio installation.
///
/// The portable MSVC toolchain used by this project deliberately omits the manifest tool. A tiny
/// activation-context file in `%TEMP%` gives only this on-demand process the same themed checkbox/
/// button classes an embedded manifest would have provided; it is removed again when the process
/// exits. DPI awareness remains programmatic, so a failed activation affects appearance only.
unsafe fn activate_common_controls() -> Option<(HANDLE, usize, PathBuf)> {
    let path = std::env::temp_dir().join("conduit-control-v6.manifest");
    if fs::write(&path, include_str!("../../conduit-control.manifest")).is_err() {
        return None;
    }
    let source: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let context = ACTCTXW {
        cbSize: std::mem::size_of::<ACTCTXW>() as u32,
        lpSource: source.as_ptr(),
        ..Default::default()
    };
    let handle = CreateActCtxW(&context);
    if handle == INVALID_HANDLE_VALUE {
        let _ = fs::remove_file(&path);
        return None;
    }
    let mut cookie = 0usize;
    if ActivateActCtx(handle, &mut cookie) == 0 {
        ReleaseActCtx(handle);
        let _ = fs::remove_file(&path);
        return None;
    }
    Some((handle, cookie, path))
}

fn config_dir() -> PathBuf {
    if let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty()) {
        return PathBuf::from(local).join("Conduit");
    }
    windows_registry::CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Shell Folders")
        .ok()
        .and_then(|key| key.get_string("Local AppData").ok())
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("Conduit")
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
    let state_label = match state.as_str() {
        "linked" | "connected" if !peer.is_empty() => peer.clone(),
        "linked" | "connected" => "Phone".to_owned(),
        "retrying" => "Reconnecting".to_owned(),
        "discovering" => "Looking for phone".to_owned(),
        _ => "Not linked".to_owned(),
    };
    let route = if path.is_empty() {
        "—".to_owned()
    } else if relay.is_empty() {
        path
    } else {
        format!("{path} · {relay}")
    };
    let detail = if daemon == "Stopped" {
        "Desktop daemon stopped".to_owned()
    } else {
        match state.as_str() {
            "linked" | "connected" if route != "—" => format!("Linked\r\n{route}"),
            "linked" | "connected" => "Linked".to_owned(),
            "retrying" if route != "—" => format!("Reconnecting\r\n{route}"),
            "discovering" => "Looking for phone".to_owned(),
            _ => "Not linked".to_owned(),
        }
    };
    set_text(ui.status_state, &state_label);
    set_text(ui.status_detail, &detail);
    let relay_source = config
        .iter()
        .find(|(key, _)| key == "relays")
        .map(|(_, value)| value.as_str())
        .unwrap_or(DEFAULT_RELAYS);
    let relay_text = relay_source
        .split([',', ';'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\r\n");
    set_text(ui.relays, &relay_text);
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
    let tray_value = value(&config, "tray_icon");
    let tray_enabled = !matches!(
        tray_value.to_ascii_lowercase().as_str(),
        "false" | "0" | "no" | "off"
    );
    SendMessageW(
        ui.tray as HWND,
        BM_SETCHECK,
        if tray_enabled { 1 } else { 0 },
        0,
    );
}

unsafe fn save_config() {
    let Some(ui) = UI.get() else { return };
    let relays = get_text(ui.relays)
        .split([',', ';', '\r', '\n'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(";");
    let proxy = get_text(ui.proxy).replace(['\r', '\n'], "");
    let tray = SendMessageW(ui.tray as HWND, BM_GETCHECK, 0, 0) == 1;
    if fs::create_dir_all(&ui.config_dir).is_err()
        || fs::write(
            ui.config_dir.join("config.txt"),
            format!("relays={relays}\nrelay_proxy={proxy}\ntray_icon={tray}\n"),
        )
        .is_err()
    {
        message("Could not save Conduit settings.", MB_ICONERROR);
        return;
    }
    message(
        "Settings saved. Restart the desktop daemon to apply Relay or tray changes.",
        MB_ICONINFORMATION,
    );
}

fn run_daemon_command(args: &[&str]) -> bool {
    let daemon = UI
        .get()
        .map(|ui| ui.daemon.clone())
        .unwrap_or_else(|| sibling("conduit-daemon.exe"));
    Command::new(daemon)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn ensure_daemon_running() {
    // Port 41112 is already the daemon's zero-extra-resource single-instance gate. Probe it once
    // when the user opens the GUI; if free, release it immediately and launch the hidden sibling.
    let Ok(probe) = std::net::TcpListener::bind(("0.0.0.0", 41112)) else {
        return;
    };
    drop(probe);
    let daemon = sibling("conduit-daemon.exe");
    if daemon.is_file() {
        let _ = Command::new(daemon)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
}

unsafe fn toggle(kind: usize) {
    let Some(ui) = UI.get() else { return };
    let hwnd = if kind == ID_AUTOSTART {
        ui.autostart
    } else {
        ui.explorer
    };
    let checked = SendMessageW(hwnd as HWND, BM_GETCHECK, 0, 0) == 1;
    let target = if kind == ID_AUTOSTART {
        "autostart"
    } else {
        "explorer"
    };
    if !run_daemon_command(&[target, if checked { "install" } else { "remove" }]) {
        message("Could not update the Windows integration.", MB_ICONERROR);
        refresh();
    }
}

unsafe fn message(text: &str, icon: MESSAGEBOX_STYLE) {
    let title = wide("Conduit");
    let text = wide(text);
    let parent = UI.get().map(|ui| ui.window as HWND).unwrap_or(null_mut());
    MessageBoxW(parent, text.as_ptr(), title.as_ptr(), MB_OK | icon);
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let Some(ui) = UI.get() else {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            };
            let mut paint: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut paint);
            let mut client: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut client);
            FillRect(hdc, &client, ui.bg_brush as HBRUSH);

            let old_pen = SelectObject(hdc, ui.border_pen as HGDIOBJ);
            let old_brush = SelectObject(hdc, ui.card_brush as HGDIOBJ);
            let radius = dip(10, ui.dpi);
            RoundRect(
                hdc,
                dip(24, ui.dpi),
                dip(78, ui.dpi),
                dip(246, ui.dpi),
                dip(496, ui.dpi),
                radius,
                radius,
            );
            RoundRect(
                hdc,
                dip(270, ui.dpi),
                dip(108, ui.dpi),
                dip(736, ui.dpi),
                dip(318, ui.dpi),
                radius,
                radius,
            );
            RoundRect(
                hdc,
                dip(270, ui.dpi),
                dip(360, ui.dpi),
                dip(736, ui.dpi),
                dip(452, ui.dpi),
                radius,
                radius,
            );
            if ui.app_mark_connection != 0 {
                DrawIconEx(
                    hdc,
                    dip(40, ui.dpi),
                    dip(116, ui.dpi),
                    ui.app_mark_connection as HICON,
                    dip(44, ui.dpi),
                    dip(44, ui.dpi),
                    0,
                    null_mut(),
                    DI_NORMAL,
                );
            }
            if ui.app_mark_title != 0 {
                DrawIconEx(
                    hdc,
                    dip(28, ui.dpi),
                    dip(20, ui.dpi),
                    ui.app_mark_title as HICON,
                    dip(34, ui.dpi),
                    dip(34, ui.dpi),
                    0,
                    null_mut(),
                    DI_NORMAL,
                );
            }
            SelectObject(hdc, old_brush);
            SelectObject(hdc, old_pen);
            EndPaint(hwnd, &paint);
            0
        }
        WM_CTLCOLORSTATIC => {
            let Some(ui) = UI.get() else {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            };
            let hdc = wparam as HDC;
            let id = GetDlgCtrlID(lparam as HWND) as usize;
            SetBkMode(wparam as HDC, TRANSPARENT as i32);
            SetTextColor(
                hdc,
                match id {
                    ID_CONNECTION_CAPTION
                    | ID_CONNECTION_DETAIL
                    | ID_RELAY_LABEL
                    | ID_PROXY_LABEL => ui.theme.muted,
                    _ => ui.theme.text,
                },
            );
            match id {
                ID_TITLE | ID_ROUTING_CAPTION | ID_INTEGRATIONS_CAPTION => ui.bg_brush as LRESULT,
                _ => ui.card_brush as LRESULT,
            }
        }
        WM_CTLCOLOREDIT => {
            let Some(ui) = UI.get() else {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            };
            SetTextColor(wparam as HDC, ui.theme.text);
            SetBkColor(wparam as HDC, ui.theme.edit);
            ui.edit_brush as LRESULT
        }
        WM_CTLCOLORBTN => {
            let Some(ui) = UI.get() else {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            };
            let id = GetDlgCtrlID(lparam as HWND) as usize;
            SetBkMode(wparam as HDC, TRANSPARENT as i32);
            SetTextColor(wparam as HDC, ui.theme.text);
            if id == ID_SAVE {
                ui.bg_brush as LRESULT
            } else {
                ui.card_brush as LRESULT
            }
        }
        WM_COMMAND => {
            let id = wparam & 0xffff;
            match id {
                ID_REFRESH => refresh(),
                ID_SAVE => save_config(),
                ID_FOLDER => {
                    if let Some(dir) = UI.get().map(|ui| ui.config_dir.clone()) {
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
        let _ = windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(
            &windows::core::HSTRING::from("Conduit.Desktop"),
        );
        ensure_daemon_running();
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let common_controls = activate_common_controls();
        let theme = app_theme();
        let bg_brush = CreateSolidBrush(theme.bg);
        let card_brush = CreateSolidBrush(theme.card);
        let edit_brush = CreateSolidBrush(theme.edit);
        let border_pen = CreatePen(PS_SOLID as i32, 1, theme.border);
        let icon_file = ensure_app_icon_file();
        let app_icon_class = icon_file
            .as_deref()
            .map(|path| load_app_icon(path, 32))
            .unwrap_or(null_mut());

        let instance = GetModuleHandleW(null());
        let class_name = wide("ConduitControlWindow");
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(wnd_proc);
        wc.hInstance = instance;
        wc.lpszClassName = class_name.as_ptr();
        wc.hIcon = app_icon_class;
        wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        wc.hbrBackground = bg_brush;
        if RegisterClassW(&wc) == 0 {
            return;
        }

        let title = wide("Conduit");
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
        let window = CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            760,
            560,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if window.is_null() {
            if !app_icon_class.is_null() {
                DestroyIcon(app_icon_class);
            }
            return;
        }

        // Load each icon at the exact physical pixel size used on this monitor. The former code
        // loaded one 48px icon and stretched it to 44 DIP/34 DIP after DPI scaling, which is why
        // the mark looked soft at 125%+ even though the ICO contained multiple frames.
        let dpi = GetDpiForWindow(window).max(96);
        let app_icon_small = icon_file
            .as_deref()
            .map(|path| load_app_icon(path, dip(16, dpi)))
            .unwrap_or(null_mut());
        let app_icon_big = icon_file
            .as_deref()
            .map(|path| load_app_icon(path, dip(32, dpi)))
            .unwrap_or(null_mut());
        let app_mark_connection = icon_file
            .as_deref()
            .map(|path| load_app_icon(path, dip(44, dpi)))
            .unwrap_or(null_mut());
        let app_mark_title = icon_file
            .as_deref()
            .map(|path| load_app_icon(path, dip(34, dpi)))
            .unwrap_or(null_mut());

        if !app_icon_big.is_null() {
            SendMessageW(
                window,
                WM_SETICON,
                ICON_BIG as usize,
                app_icon_big as LPARAM,
            );
        }
        if !app_icon_small.is_null() {
            SendMessageW(
                window,
                WM_SETICON,
                ICON_SMALL as usize,
                app_icon_small as LPARAM,
            );
        }

        // Derive every coordinate from the actual HWND DPI so parent and children stay in the same
        // coordinate space at 125%+ scaling. The UI remains fixed-size and on-demand; no layout
        // watcher or resize loop is introduced.
        let mut bounds = RECT {
            left: 0,
            top: 0,
            right: dip(760, dpi),
            bottom: dip(560, dpi),
        };
        AdjustWindowRectExForDpi(&mut bounds, style, 0, 0, dpi);
        SetWindowPos(
            window,
            null_mut(),
            0,
            0,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );

        let body_font = font(dpi, 14, FW_NORMAL as i32);
        let caption_font = font(dpi, 12, FW_SEMIBOLD as i32);
        let title_font = font(dpi, 28, FW_SEMIBOLD as i32);
        let state_font = font(dpi, 24, FW_SEMIBOLD as i32);
        let peer_font = font(dpi, 18, FW_SEMIBOLD as i32);

        let dark = if theme.dark { 1i32 } else { 0i32 };
        DwmSetWindowAttribute(
            window,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            &dark as *const _ as *const _,
            std::mem::size_of_val(&dark) as u32,
        );
        let corner = DWMWCP_ROUND;
        DwmSetWindowAttribute(
            window,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &corner as *const _ as *const _,
            std::mem::size_of_val(&corner) as u32,
        );
        set_native_theme(window, theme.dark);

        let label = |text: &str, id: usize, x, y, w, h, font_handle: HFONT| {
            let hwnd = child(
                "STATIC",
                text,
                STATIC_NO_PREFIX,
                dip(x, dpi),
                dip(y, dpi),
                dip(w, dpi),
                dip(h, dpi),
                window,
                id,
                instance,
            );
            set_font(hwnd, font_handle);
            hwnd
        };

        label("Conduit", ID_TITLE, 76, 18, 300, 40, title_font);

        // Left pane: connection state only. No explanatory copy or transport implementation detail.
        label(
            "Connection",
            ID_CONNECTION_CAPTION,
            44,
            96,
            150,
            20,
            caption_font,
        );
        let status_state = label(
            "Not linked",
            ID_CONNECTION_STATE,
            96,
            116,
            126,
            58,
            peer_font,
        );
        let status_detail = label(
            "Not linked",
            ID_CONNECTION_DETAIL,
            44,
            184,
            174,
            48,
            body_font,
        );

        // Right pane: settings are grouped like Windows Settings cards, with short labels only.
        label("Relay", ID_ROUTING_CAPTION, 290, 78, 220, 26, state_font);
        label("Endpoints", ID_RELAY_LABEL, 290, 126, 160, 22, body_font);
        let relays = child(
            "EDIT",
            "",
            WS_BORDER | WS_TABSTOP | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32,
            dip(290, dpi),
            dip(150, dpi),
            dip(426, dpi),
            dip(62, dpi),
            window,
            0,
            instance,
        );
        label("SOCKS5 proxy", ID_PROXY_LABEL, 290, 224, 180, 22, body_font);
        let proxy = child(
            "EDIT",
            "",
            WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
            dip(290, dpi),
            dip(248, dpi),
            dip(426, dpi),
            dip(34, dpi),
            window,
            0,
            instance,
        );

        label(
            "Windows",
            ID_INTEGRATIONS_CAPTION,
            290,
            330,
            220,
            26,
            state_font,
        );
        let autostart = child(
            "BUTTON",
            "Start at sign-in",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            dip(290, dpi),
            dip(382, dpi),
            dip(190, dpi),
            dip(24, dpi),
            window,
            ID_AUTOSTART,
            instance,
        );
        let explorer = child(
            "BUTTON",
            "Send to phone in Explorer",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            dip(500, dpi),
            dip(382, dpi),
            dip(206, dpi),
            dip(24, dpi),
            window,
            ID_EXPLORER,
            instance,
        );
        let tray = child(
            "BUTTON",
            "Show tray icon",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            dip(290, dpi),
            dip(420, dpi),
            dip(190, dpi),
            dip(24, dpi),
            window,
            ID_TRAY,
            instance,
        );

        // Actions stay explicit and on-demand. Ampersands provide native access keys.
        let folder = child(
            "BUTTON",
            "&Diagnostics",
            BS_PUSHBUTTON as u32 | WS_TABSTOP,
            dip(44, dpi),
            dip(404, dpi),
            dip(158, dpi),
            dip(36, dpi),
            window,
            ID_FOLDER,
            instance,
        );
        let refresh_button = child(
            "BUTTON",
            "&Refresh",
            BS_PUSHBUTTON as u32 | WS_TABSTOP,
            dip(44, dpi),
            dip(448, dpi),
            dip(158, dpi),
            dip(36, dpi),
            window,
            ID_REFRESH,
            instance,
        );
        let save = child(
            "BUTTON",
            "&Save",
            BS_DEFPUSHBUTTON as u32 | WS_TABSTOP,
            dip(612, dpi),
            dip(486, dpi),
            dip(124, dpi),
            dip(36, dpi),
            window,
            ID_SAVE,
            instance,
        );

        for hwnd in [
            relays,
            proxy,
            autostart,
            explorer,
            tray,
            refresh_button,
            save,
            folder,
        ] {
            set_font(hwnd, body_font);
            set_native_theme(hwnd, theme.dark);
        }

        UI.set(Ui {
            window: window as isize,
            status_state: status_state as isize,
            status_detail: status_detail as isize,
            relays: relays as isize,
            proxy: proxy as isize,
            autostart: autostart as isize,
            explorer: explorer as isize,
            tray: tray as isize,
            config_dir: config_dir(),
            daemon: sibling("conduit-daemon.exe"),
            dpi,
            theme,
            bg_brush: bg_brush as isize,
            card_brush: card_brush as isize,
            edit_brush: edit_brush as isize,
            border_pen: border_pen as isize,
            app_mark_connection: app_mark_connection as isize,
            app_mark_title: app_mark_title as isize,
        })
        .ok();

        refresh();
        InvalidateRect(window, null(), 1);
        ShowWindow(window, SW_SHOW);
        UpdateWindow(window);

        // Give a normal top-level Win32 window dialog-style keyboard traversal without turning it
        // into a dialog resource or adding another framework. This handles Tab/Shift+Tab and native
        // access/default-button behavior while keeping the process entirely on-demand.
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(window, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }

        if let Some((handle, cookie, path)) = common_controls {
            DeactivateActCtx(0, cookie);
            ReleaseActCtx(handle);
            let _ = fs::remove_file(path);
        }
        for icon in [
            app_icon_class,
            app_icon_small,
            app_icon_big,
            app_mark_connection,
            app_mark_title,
        ] {
            if !icon.is_null() {
                DestroyIcon(icon);
            }
        }
        for object in [
            body_font as HGDIOBJ,
            caption_font as HGDIOBJ,
            title_font as HGDIOBJ,
            state_font as HGDIOBJ,
            peer_font as HGDIOBJ,
            bg_brush as HGDIOBJ,
            card_brush as HGDIOBJ,
            edit_brush as HGDIOBJ,
            border_pen as HGDIOBJ,
        ] {
            DeleteObject(object);
        }
    }
}
