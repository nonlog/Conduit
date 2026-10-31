//! Rune does not automatically apply Windows PE icon resources to its winit
//! window. Assign both WM_SETICON sizes explicitly so title bar, Alt+Tab and
//! taskbar display Conduit's real icon instead of the generic application glyph.

#![cfg(windows)]

use rune::raw_window_handle::RawWindowHandle;
use std::collections::HashMap;
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_LOADFROMFILE, LoadImageW, SendMessageW,
    WM_SETICON,
};
use winreg::{RegKey, enums::HKEY_CURRENT_USER};

#[derive(Clone, Copy)]
struct InstalledIcons {
    handles: [isize; 2],
    dark: bool,
}

static ICONS: OnceLock<Mutex<HashMap<isize, InstalledIcons>>> = OnceLock::new();

fn loaded_icons() -> &'static Mutex<HashMap<isize, InstalledIcons>> {
    ICONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn system_dark() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|key| key.get_value::<u32, _>("AppsUseLightTheme"))
        .map(|value| value == 0)
        .unwrap_or(false)
}

fn effective_dark(mode: &str) -> bool {
    match mode {
        "dark" => true,
        "light" => false,
        _ => system_dark(),
    }
}

fn set_dark_caption(hwnd: *mut core::ffi::c_void, dark: bool) -> bool {
    // Native Windows caption must follow the Rune appearance override, not
    // the user's unrelated Windows AppsUseLightTheme setting.
    let dark = if dark { 1i32 } else { 0i32 };
    let hr = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const i32 as *const core::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        )
    };
    hr >= 0
}

fn apply_to_hwnd(hwnd: *mut core::ffi::c_void, requested_dark: bool) -> Result<(), String> {
    // Keep the correct small/large window icons if older Windows versions do
    // not support the immersive DWM caption attribute.
    let dark = if set_dark_caption(hwnd, requested_dark) {
        requested_dark
    } else {
        system_dark()
    };
    let key = hwnd as isize;
    let mut cache = loaded_icons().lock().map_err(|err| err.to_string())?;
    if cache.get(&key).is_some_and(|current| current.dark == dark) {
        return Ok(());
    }

    let path = std::env::current_exe()
        .map_err(|err| err.to_string())?
        .with_file_name(if dark {
            "conduit-icon-dark.ico"
        } else {
            "conduit-icon-light.ico"
        });
    let wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let small = ((16 * dpi + 95) / 96).clamp(16, 64) as i32;
    let big = ((32 * dpi + 95) / 96).clamp(32, 128) as i32;
    let instance = unsafe { GetModuleHandleW(null()) };
    let load = |size| unsafe {
        let mut icon = if path.is_file() {
            LoadImageW(
                null_mut(),
                wide_path.as_ptr(),
                IMAGE_ICON,
                size,
                size,
                LR_LOADFROMFILE,
            )
        } else {
            null_mut()
        };
        // Self-contained preview builds can use the embedded icon resource.
        if icon.is_null() {
            icon = LoadImageW(instance, 1usize as *const u16, IMAGE_ICON, size, size, 0);
        }
        icon
    };
    let small_icon = load(small);
    let big_icon = load(big);
    if small_icon.is_null() || big_icon.is_null() {
        unsafe {
            if !small_icon.is_null() {
                DestroyIcon(small_icon);
            }
            if !big_icon.is_null() {
                DestroyIcon(big_icon);
            }
        }
        return Err(format!(
            "Loading Conduit window icon: {}",
            std::io::Error::last_os_error()
        ));
    }
    unsafe {
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, small_icon as isize);
        SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, big_icon as isize);
    }
    if let Some(previous) = cache.insert(
        key,
        InstalledIcons {
            handles: [small_icon as isize, big_icon as isize],
            dark,
        },
    ) {
        unsafe {
            for icon in previous.handles {
                if icon != 0 {
                    DestroyIcon(icon as *mut core::ffi::c_void);
                }
            }
        }
    }
    Ok(())
}

/// Called for each new Rune HWND; both the DWM caption and the small/large
/// icons must match the actual Conduit appearance rather than Windows' theme.
pub fn install(window: &rune::WindowHandle) -> Result<(), String> {
    window.with_raw_handle(|handle| {
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err("Conduit window is not a Win32 HWND".into());
        };
        apply_to_hwnd(
            handle.hwnd.get() as *mut core::ffi::c_void,
            effective_dark(&crate::bridge::theme_preference()),
        )
    })?
}

/// Switching Light/Dark/System in Settings updates Windows chrome immediately
/// without restarting the UI or leaking previously installed HICON handles.
pub fn update_appearance(mode: &str) -> Result<(), String> {
    let handles: Vec<isize> = loaded_icons()
        .lock()
        .map_err(|err| err.to_string())?
        .keys()
        .copied()
        .collect();
    let dark = effective_dark(mode);
    for hwnd in handles {
        apply_to_hwnd(hwnd as *mut core::ffi::c_void, dark)?;
    }
    Ok(())
}

/// Called after Rune's window has been destroyed, so no HWND retains an icon.
pub fn release() {
    if let Some(cache) = ICONS.get() {
        if let Ok(mut cache) = cache.lock() {
            for icons in cache.drain().map(|(_, pair)| pair.handles) {
                unsafe {
                    for icon in icons {
                        if icon != 0 {
                            DestroyIcon(icon as *mut core::ffi::c_void);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::effective_dark;

    #[test]
    fn explicit_theme_controls_native_caption_independently_of_windows_theme() {
        assert!(effective_dark("dark"));
        assert!(!effective_dark("light"));
    }
}
