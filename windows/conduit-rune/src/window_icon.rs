//! Rune does not automatically apply Windows PE icon resources to its winit
//! window. Assign both WM_SETICON sizes explicitly so title bar, Alt+Tab and
//! taskbar display Conduit's real icon instead of the generic application glyph.

#![cfg(windows)]

use rune::raw_window_handle::RawWindowHandle;
use std::collections::HashMap;
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_LOADFROMFILE, LoadImageW, SendMessageW,
    WM_SETICON,
};

static ICONS: OnceLock<Mutex<HashMap<isize, [isize; 2]>>> = OnceLock::new();

fn loaded_icons() -> &'static Mutex<HashMap<isize, [isize; 2]>> {
    ICONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Called on the Rune UI thread after each native HWND is created. A repeated
/// on-ready callback must not allocate another pair of HICON handles.
pub fn install(window: &rune::WindowHandle) -> Result<(), String> {
    window.with_raw_handle(|handle| {
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err("Conduit window is not a Win32 HWND".into());
        };
        let hwnd = handle.hwnd.get() as *mut core::ffi::c_void;
        let key = hwnd as isize;
        let mut cache = loaded_icons().lock().map_err(|err| err.to_string())?;
        if cache.contains_key(&key) {
            return Ok(());
        }

        // Use the multi-resolution ICO shipped next to conduit-rune.exe, which
        // has native 16–64px frames for high-DPI Windows title bars.
        let path = std::env::current_exe()
            .map_err(|err| err.to_string())?
            .with_file_name("conduit-icon.ico");
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
            // Isolated Rune preview ZIPs do not include the sibling ICO. Its
            // winres build embeds resource ID 1 into the GUI executable.
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
        cache.insert(key, [small_icon as isize, big_icon as isize]);
        Ok(())
    })?
}

/// Called after Rune's window has been destroyed, so no HWND retains an icon.
pub fn release() {
    if let Some(cache) = ICONS.get() {
        if let Ok(mut cache) = cache.lock() {
            for icons in cache.drain().map(|(_, handles)| handles) {
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
