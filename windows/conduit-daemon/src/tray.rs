//! Optional Windows notification-area icon for the already-resident daemon.
//!
//! The tray is deliberately one blocked Win32 message loop: no timer, no polling, no watcher and
//! no status refresh traffic. When disabled in config, this module starts no thread at all.

use anyhow::{bail, Context, Result};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::{mpsc, OnceLock};
use std::thread;
use std::time::Duration;
use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, PROCESS_INFORMATION, STARTF_FORCEOFFFEEDBACK, STARTUPINFOW,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CALLBACK_MESSAGE: u32 = WM_APP + 1;
const TRAY_ID: u32 = 1;
const MENU_OPEN: usize = 1001;
const MENU_EXIT: usize = 1002;
const TRAY_DARK_BYTES: &[u8] = include_bytes!("../assets/conduit-tray-dark.ico");
const TRAY_LIGHT_BYTES: &[u8] = include_bytes!("../assets/conduit-tray-light.ico");

static CONTROL_PATH: OnceLock<PathBuf> = OnceLock::new();
static CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();
static EXIT_TX: OnceLock<tokio::sync::mpsc::UnboundedSender<()>> = OnceLock::new();
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static TRAY_ICON: AtomicIsize = AtomicIsize::new(0);

pub struct Tray {
    thread: Option<thread::JoinHandle<()>>,
    hwnd: isize,
}

impl Tray {
    pub fn start(
        config_dir: &Path,
        exit_tx: tokio::sync::mpsc::UnboundedSender<()>,
    ) -> Result<Self> {
        let control = control_path()?;
        let _ = CONTROL_PATH.set(control);
        let _ = CONFIG_DIR.set(config_dir.to_path_buf());
        let _ = EXIT_TX.set(exit_tx);
        ensure_tray_assets(config_dir)?;
        let (ready_tx, ready_rx) = mpsc::sync_channel::<std::result::Result<isize, String>>(1);
        let handle = thread::Builder::new()
            .name("conduit-tray".into())
            .spawn(move || unsafe {
                if let Err(e) = run(ready_tx.clone()) {
                    let _ = ready_tx.try_send(Err(format!("{e:#}")));
                }
            })
            .context("starting Conduit tray thread")?;
        match ready_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(hwnd)) => Ok(Self {
                thread: Some(handle),
                hwnd,
            }),
            Ok(Err(message)) => bail!("starting Conduit tray: {message}"),
            Err(_) => bail!("timed out starting Conduit tray"),
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            if self.hwnd != 0 {
                let _ = PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0);
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn ensure_tray_assets(config_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(config_dir)
        .with_context(|| format!("creating {}", config_dir.display()))?;
    for (name, bytes) in [
        ("conduit-tray-dark.ico", TRAY_DARK_BYTES),
        ("conduit-tray-light.ico", TRAY_LIGHT_BYTES),
    ] {
        let path = config_dir.join(name);
        let matches = std::fs::read(&path)
            .ok()
            .is_some_and(|current| current.as_slice() == bytes);
        if !matches {
            std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
        }
    }
    Ok(())
}

fn system_uses_light_theme() -> bool {
    windows_registry::CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .ok()
        .and_then(|key| key.get_u32("SystemUsesLightTheme").ok())
        .unwrap_or(1)
        != 0
}

fn tray_icon_path() -> Option<PathBuf> {
    let dir = CONFIG_DIR.get()?;
    Some(dir.join(if system_uses_light_theme() {
        "conduit-tray-light.ico"
    } else {
        "conduit-tray-dark.ico"
    }))
}

fn control_path() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("finding Conduit executable")?;
    let dir = exe.parent().context("Conduit executable has no parent")?;
    let installed = dir.join("Conduit.exe");
    if installed.is_file() {
        return Ok(installed);
    }
    Ok(dir.join("conduit-control.exe"))
}

fn wide(value: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    value
        .as_ref()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

unsafe fn data(hwnd: HWND, icon: HICON) -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = std::mem::zeroed();
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = TRAY_ID;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    data.uCallbackMessage = CALLBACK_MESSAGE;
    data.hIcon = icon;
    let tip = wide("Conduit");
    let count = tip.len().min(data.szTip.len());
    data.szTip[..count].copy_from_slice(&tip[..count]);
    data
}

unsafe fn add_icon(hwnd: HWND, icon: HICON) -> bool {
    Shell_NotifyIconW(NIM_ADD, &data(hwnd, icon)) != 0
}

unsafe fn modify_icon(hwnd: HWND, icon: HICON) -> bool {
    Shell_NotifyIconW(NIM_MODIFY, &data(hwnd, icon)) != 0
}

unsafe fn set_modern_version(hwnd: HWND, icon: HICON) {
    let mut version = data(hwnd, icon);
    version.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    let _ = Shell_NotifyIconW(NIM_SETVERSION, &version);
}

unsafe fn load_tray_icon() -> Result<HICON> {
    let path = tray_icon_path().context("Conduit tray icon directory is unavailable")?;
    let path = wide(path.as_os_str());
    // Map the canonical 16 px notification-area size to physical pixels. The renderer emits
    // matching 16/20/24/28/32/40/48/64 ICO frames, so Explorer selects a native frame instead
    // of upscaling a soft 16 px bitmap. Avoid GetSystemMetricsForDpi here because the windows-sys
    // version used by Conduit does not expose that Win32 symbol.
    let dpi = GetDpiForSystem().max(96);
    let size = (((16u32 * dpi) + 95) / 96).clamp(16, 64) as i32;
    let icon = LoadImageW(
        null_mut(),
        path.as_ptr(),
        IMAGE_ICON,
        size,
        size,
        LR_LOADFROMFILE,
    ) as HICON;
    if icon.is_null() {
        bail!(
            "loading tray icon failed: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(icon)
}

unsafe fn install_window_icon(hwnd: HWND, icon: HICON) {
    let _ = SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, icon as isize);
    let _ = SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, icon as isize);
}

unsafe fn refresh_tray_icon(hwnd: HWND) {
    let Ok(icon) = load_tray_icon() else { return };
    if !modify_icon(hwnd, icon) {
        DestroyIcon(icon);
        return;
    }
    install_window_icon(hwnd, icon);
    let old = TRAY_ICON.swap(icon as isize, Ordering::Relaxed) as HICON;
    if !old.is_null() && old != icon {
        DestroyIcon(old);
    }
}

unsafe fn run(ready: mpsc::SyncSender<std::result::Result<isize, String>>) -> Result<()> {
    let instance = GetModuleHandleW(null());
    let class = wide("ConduitTrayWindow");
    let icon = load_tray_icon()?;
    let mut wc: WNDCLASSW = std::mem::zeroed();
    wc.lpfnWndProc = Some(wnd_proc);
    wc.hInstance = instance;
    wc.hIcon = icon;
    wc.lpszClassName = class.as_ptr();
    if RegisterClassW(&wc) == 0 && std::io::Error::last_os_error().raw_os_error() != Some(1410) {
        DestroyIcon(icon);
        bail!("RegisterClassW failed: {}", std::io::Error::last_os_error());
    }

    let hwnd = CreateWindowExW(
        0,
        class.as_ptr(),
        class.as_ptr(),
        0,
        0,
        0,
        0,
        0,
        null_mut(),
        null_mut(),
        instance,
        null(),
    );
    if hwnd.is_null() {
        DestroyIcon(icon);
        bail!(
            "CreateWindowExW failed: {}",
            std::io::Error::last_os_error()
        );
    }
    install_window_icon(hwnd, icon);
    TRAY_ICON.store(icon as isize, Ordering::Relaxed);
    let taskbar_message = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
    TASKBAR_CREATED.store(taskbar_message, Ordering::Relaxed);

    if !add_icon(hwnd, icon) {
        DestroyIcon(icon);
        DestroyWindow(hwnd);
        bail!("Shell_NotifyIconW(NIM_ADD) failed");
    }
    set_modern_version(hwnd, icon);
    let _ = ready.send(Ok(hwnd as isize));

    let mut message: MSG = std::mem::zeroed();
    while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }

    let current = TRAY_ICON.swap(0, Ordering::Relaxed) as HICON;
    let _ = Shell_NotifyIconW(NIM_DELETE, &data(hwnd, current));
    if !current.is_null() {
        DestroyIcon(current);
    }
    Ok(())
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let taskbar = TASKBAR_CREATED.load(Ordering::Relaxed);
    if taskbar != 0 && msg == taskbar {
        let icon = TRAY_ICON.load(Ordering::Relaxed) as HICON;
        if !icon.is_null() {
            let _ = add_icon(hwnd, icon);
            set_modern_version(hwnd, icon);
        }
        return 0;
    }

    match msg {
        CALLBACK_MESSAGE => {
            // With NOTIFYICON_VERSION_4 the low word is the mouse/message event and the high word
            // contains the icon id. Masking also remains compatible with the legacy callback form.
            match (lparam as u32) & 0xffff {
                WM_LBUTTONDBLCLK => open_control(),
                WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
                _ => {}
            }
            0
        }
        WM_COMMAND => {
            match wparam & 0xffff {
                MENU_OPEN => open_control(),
                MENU_EXIT => request_exit(hwnd),
                _ => {}
            }
            0
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED => {
            refresh_tray_icon(hwnd);
            if let Err(e) = crate::explorer::refresh_icon() {
                tracing::warn!(error = %e, "could not refresh Explorer icon for theme change");
            }
            if let Some(dir) = CONFIG_DIR.get() {
                if let Err(e) = crate::toast::refresh_app_identity(dir) {
                    tracing::warn!(error = %e, "could not refresh notification identity icon for theme change");
                }
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

unsafe fn show_menu(hwnd: HWND) {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    let open = wide("Open Conduit");
    let exit = wide("Exit Conduit");
    let _ = AppendMenuW(menu, MF_STRING, MENU_OPEN, open.as_ptr());
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, null());
    let _ = AppendMenuW(menu, MF_STRING, MENU_EXIT, exit.as_ptr());

    let mut point = POINT::default();
    if GetCursorPos(&mut point) != 0 {
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, point.x, point.y, 0, hwnd, null());
        let _ = PostMessageW(hwnd, WM_NULL, 0, 0);
    }
    DestroyMenu(menu);
}

unsafe fn request_exit(hwnd: HWND) {
    let icon = TRAY_ICON.load(Ordering::Relaxed) as HICON;
    if !icon.is_null() {
        let _ = Shell_NotifyIconW(NIM_DELETE, &data(hwnd, icon));
    }
    if let Some(tx) = EXIT_TX.get() {
        let _ = tx.send(());
    }
    DestroyWindow(hwnd);
}

unsafe fn open_control() {
    // Keep compatibility with the legacy control window, but prefer the real WinUI title for the
    // current shell. A second tray activation should restore the existing UI immediately instead
    // of launching another Uno/WinUI process.
    let legacy_class = wide("ConduitControlWindow");
    let mut existing = FindWindowW(legacy_class.as_ptr(), null());
    if existing.is_null() {
        let title = wide("Conduit");
        existing = FindWindowW(null(), title.as_ptr());
    }
    if !existing.is_null() {
        ShowWindow(existing, SW_RESTORE);
        let _ = SetForegroundWindow(existing);
        return;
    }
    if let Some(path) = CONTROL_PATH.get() {
        if let Err(e) = launch_control(path) {
            tracing::warn!(error = %e, "could not launch Conduit UI from tray");
        }
    }
}

unsafe fn launch_control(path: &Path) -> Result<()> {
    let application = wide(path.as_os_str());
    let mut startup = STARTUPINFOW::default();
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    // A GUI process gets the Windows launch-feedback (busy) cursor by default. Conduit reaches its
    // first window in well under a second, so that feedback makes a fast launch feel much slower
    // than it is. Force the normal pointer while retaining cold-start-on-demand memory behaviour.
    startup.dwFlags = STARTF_FORCEOFFFEEDBACK;
    let mut process = PROCESS_INFORMATION::default();
    if CreateProcessW(
        application.as_ptr(),
        null_mut(),
        null(),
        null(),
        0,
        0,
        null(),
        null(),
        &startup,
        &mut process,
    ) == 0
    {
        bail!(
            "CreateProcessW({}) failed: {}",
            path.display(),
            std::io::Error::last_os_error()
        );
    }
    if !process.hThread.is_null() {
        CloseHandle(process.hThread);
    }
    if !process.hProcess.is_null() {
        CloseHandle(process.hProcess);
    }
    Ok(())
}
