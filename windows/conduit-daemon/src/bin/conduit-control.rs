#![windows_subsystem = "windows"]

//! Small, on-demand Windows control surface.
//!
//! It owns no transport, tray icon, worker thread or timer. Opening the window reads the daemon's
//! event-written `status.txt` and `config.txt`; Refresh does the same on demand. Closing the window
//! ends the process completely.

use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
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
const ID_TITLE: usize = 201;
const ID_SUBTITLE: usize = 202;
const ID_CONNECTION_CAPTION: usize = 203;
const ID_CONNECTION_STATE: usize = 204;
const ID_CONNECTION_DETAIL: usize = 205;
const ID_ROUTING_CAPTION: usize = 206;
const ID_RELAY_LABEL: usize = 207;
const ID_PROXY_LABEL: usize = 208;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const STATIC_NO_PREFIX: u32 = 0x0080;

#[derive(Clone, Copy)]
struct Theme {
    dark: bool,
    bg: COLORREF,
    card: COLORREF,
    edit: COLORREF,
    text: COLORREF,
    muted: COLORREF,
    accent: COLORREF,
}

struct Ui {
    window: isize,
    status_state: isize,
    status_detail: isize,
    relays: isize,
    proxy: isize,
    autostart: isize,
    explorer: isize,
    config_dir: PathBuf,
    daemon: PathBuf,
    dpi: u32,
    theme: Theme,
    bg_brush: isize,
    card_brush: isize,
    edit_brush: isize,
    accent_brush: isize,
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
    let accent = unsafe { GetSysColor(COLOR_HIGHLIGHT) };
    if light {
        Theme {
            dark: false,
            bg: rgb(243, 243, 243),
            card: rgb(255, 255, 255),
            edit: rgb(255, 255, 255),
            text: rgb(31, 31, 31),
            muted: rgb(96, 96, 96),
            accent,
        }
    } else {
        Theme {
            dark: true,
            bg: rgb(32, 32, 32),
            card: rgb(44, 44, 44),
            edit: rgb(50, 50, 50),
            text: rgb(245, 245, 245),
            muted: rgb(184, 184, 184),
            accent,
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
    let name = wide(if dark { "DarkMode_Explorer" } else { "Explorer" });
    SetWindowTheme(hwnd, name.as_ptr(), null());
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
        "linked" | "connected" if !peer.is_empty() => format!("Linked to {peer}"),
        "linked" | "connected" => "Linked".to_owned(),
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
    set_text(ui.status_state, &state_label);
    set_text(
        ui.status_detail,
        &format!("Desktop daemon  {daemon}\r\nRoute  {route}"),
    );
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
        .map(|ui| ui.daemon.clone())
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
    let hwnd = if kind == ID_AUTOSTART { ui.autostart } else { ui.explorer };
    let checked = SendMessageW(hwnd as HWND, BM_GETCHECK, 0, 0) == 1;
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
        .map(|ui| ui.window as HWND)
        .unwrap_or(null_mut());
    MessageBoxW(parent, text.as_ptr(), title.as_ptr(), MB_OK | icon);
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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

            let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
            let old_brush = SelectObject(hdc, ui.card_brush as HGDIOBJ);
            let radius = dip(12, ui.dpi);
            RoundRect(
                hdc,
                dip(24, ui.dpi),
                dip(90, ui.dpi),
                dip(616, ui.dpi),
                dip(216, ui.dpi),
                radius,
                radius,
            );
            RoundRect(
                hdc,
                dip(24, ui.dpi),
                dip(232, ui.dpi),
                dip(616, ui.dpi),
                dip(464, ui.dpi),
                radius,
                radius,
            );
            SelectObject(hdc, ui.accent_brush as HGDIOBJ);
            RoundRect(
                hdc,
                dip(36, ui.dpi),
                dip(108, ui.dpi),
                dip(40, ui.dpi),
                dip(198, ui.dpi),
                dip(4, ui.dpi),
                dip(4, ui.dpi),
            );
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
                    ID_SUBTITLE | ID_CONNECTION_CAPTION | ID_CONNECTION_DETAIL | ID_ROUTING_CAPTION => {
                        ui.theme.muted
                    }
                    ID_CONNECTION_STATE => ui.theme.accent,
                    _ => ui.theme.text,
                },
            );
            match id {
                ID_TITLE | ID_SUBTITLE => ui.bg_brush as LRESULT,
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
            SetBkMode(wparam as HDC, TRANSPARENT as i32);
            SetTextColor(wparam as HDC, ui.theme.text);
            ui.card_brush as LRESULT
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
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let common_controls = activate_common_controls();
        let theme = app_theme();
        let bg_brush = CreateSolidBrush(theme.bg);
        let card_brush = CreateSolidBrush(theme.card);
        let edit_brush = CreateSolidBrush(theme.edit);
        let accent_brush = CreateSolidBrush(theme.accent);

        let instance = GetModuleHandleW(null());
        let class_name = wide("ConduitControlWindow");
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(wnd_proc);
        wc.hInstance = instance;
        wc.lpszClassName = class_name.as_ptr();
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
            640,
            548,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if window.is_null() {
            return;
        }

        // Ask the real window which monitor DPI it landed on. `GetDpiForSystem` deliberately
        // returns a virtualized value for per-monitor-aware callers on some Windows builds, which
        // caused the first Fluent prototype to size the parent at 96 DPI while scaling its child
        // controls for 120 DPI. Deriving everything from the actual HWND keeps one coordinate space.
        let dpi = GetDpiForWindow(window).max(96);
        let mut bounds = RECT {
            left: 0,
            top: 0,
            right: dip(640, dpi),
            bottom: dip(548, dpi),
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
        let title_font = font(dpi, 26, FW_SEMIBOLD as i32);
        let state_font = font(dpi, 20, FW_SEMIBOLD as i32);

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
        label("Conduit", ID_TITLE, 28, 18, 420, 40, title_font);
        label(
            "Phone companion · low-overhead sync",
            ID_SUBTITLE,
            30,
            60,
            480,
            22,
            body_font,
        );
        label(
            "CONNECTION",
            ID_CONNECTION_CAPTION,
            52,
            104,
            200,
            20,
            caption_font,
        );
        let status_state = label(
            "Not linked",
            ID_CONNECTION_STATE,
            52,
            128,
            520,
            34,
            state_font,
        );
        let status_detail = label(
            "Desktop daemon  —\r\nRoute  —",
            ID_CONNECTION_DETAIL,
            52,
            166,
            520,
            42,
            body_font,
        );
        label(
            "ROUTING & INTEGRATIONS",
            ID_ROUTING_CAPTION,
            44,
            246,
            260,
            20,
            caption_font,
        );
        label(
            "Relay endpoints",
            ID_RELAY_LABEL,
            44,
            276,
            180,
            22,
            body_font,
        );
        let relays = child(
            "EDIT",
            "",
            WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
            dip(44, dpi),
            dip(300, dpi),
            dip(552, dpi),
            dip(34, dpi),
            window,
            0,
            instance,
        );
        label(
            "Relay SOCKS5 proxy · blank = direct",
            ID_PROXY_LABEL,
            44,
            346,
            340,
            22,
            body_font,
        );
        let proxy = child(
            "EDIT",
            "",
            WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
            dip(44, dpi),
            dip(370, dpi),
            dip(552, dpi),
            dip(34, dpi),
            window,
            0,
            instance,
        );
        let autostart = child(
            "BUTTON",
            "Start Conduit when I sign in",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            dip(44, dpi),
            dip(420, dpi),
            dip(250, dpi),
            dip(24, dpi),
            window,
            ID_AUTOSTART,
            instance,
        );
        let explorer = child(
            "BUTTON",
            "Show Send to phone in Explorer",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            dip(318, dpi),
            dip(420, dpi),
            dip(270, dpi),
            dip(24, dpi),
            window,
            ID_EXPLORER,
            instance,
        );
        let folder = child(
            "BUTTON",
            "Open diagnostics",
            BS_PUSHBUTTON as u32 | WS_TABSTOP,
            dip(24, dpi),
            dip(490, dpi),
            dip(142, dpi),
            dip(36, dpi),
            window,
            ID_FOLDER,
            instance,
        );
        let refresh_button = child(
            "BUTTON",
            "Refresh",
            BS_PUSHBUTTON as u32 | WS_TABSTOP,
            dip(348, dpi),
            dip(490, dpi),
            dip(112, dpi),
            dip(36, dpi),
            window,
            ID_REFRESH,
            instance,
        );
        let save = child(
            "BUTTON",
            "Save settings",
            BS_DEFPUSHBUTTON as u32 | WS_TABSTOP,
            dip(472, dpi),
            dip(490, dpi),
            dip(144, dpi),
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
            config_dir: config_dir(),
            daemon: sibling("conduit-daemon.exe"),
            dpi,
            theme,
            bg_brush: bg_brush as isize,
            card_brush: card_brush as isize,
            edit_brush: edit_brush as isize,
            accent_brush: accent_brush as isize,
        })
        .ok();

        refresh();
        InvalidateRect(window, null(), 1);
        ShowWindow(window, SW_SHOW);
        UpdateWindow(window);

        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        if let Some((handle, cookie, path)) = common_controls {
            DeactivateActCtx(0, cookie);
            ReleaseActCtx(handle);
            let _ = fs::remove_file(path);
        }
        for object in [
            body_font as HGDIOBJ,
            caption_font as HGDIOBJ,
            title_font as HGDIOBJ,
            state_font as HGDIOBJ,
            bg_brush as HGDIOBJ,
            card_brush as HGDIOBJ,
            edit_brush as HGDIOBJ,
            accent_brush as HGDIOBJ,
        ] {
            DeleteObject(object);
        }
    }
}
