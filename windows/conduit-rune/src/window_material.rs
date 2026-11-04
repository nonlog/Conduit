//! Native Windows backdrop, not a blurred screenshot or a permanently running effect.
//! DWM owns composition; Rune only paints translucent, readable surfaces above it.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Material {
    #[default]
    Acrylic,
    Mica,
    Solid,
}

impl Material {
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "mica" => Self::Mica,
            "solid" => Self::Solid,
            _ => Self::Acrylic,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Acrylic => "acrylic",
            Self::Mica => "mica",
            Self::Solid => "solid",
        }
    }

    fn backdrop(self, permitted: bool) -> i32 {
        // DWM_SYSTEMBACKDROP_TYPE: NONE, MAINWINDOW (Mica), TRANSIENTWINDOW (Acrylic).
        match (self, permitted) {
            (_, false) | (Self::Solid, _) => 1,
            (Self::Mica, true) => 2,
            (Self::Acrylic, true) => 3,
        }
    }
}

#[cfg(windows)]
mod native {
    use super::Material;
    use rune::raw_window_handle::RawWindowHandle;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmExtendFrameIntoClientArea,
        DwmSetWindowAttribute,
    };
    use windows_sys::Win32::UI::Controls::MARGINS;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, SystemParametersInfoW,
        WS_EX_NOREDIRECTIONBITMAP,
    };
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};

    static WINDOW: AtomicIsize = AtomicIsize::new(0);

    /// Rune presents a transparent DirectComposition visual. A legacy HWND
    /// redirection bitmap underneath it can hide DWM's backdrop behind an
    /// opaque white surface, even when the backdrop attribute reports success.
    /// Configure our own HWND before its first frame; never change another app.
    unsafe fn prepare_composition(hwnd: *mut core::ffi::c_void) -> bool {
        unsafe {
            let before = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let required = WS_EX_NOREDIRECTIONBITMAP as isize;
            if before & required != 0 {
                return true;
            }
            SetLastError(0);
            let previous = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, before | required);
            if previous == 0 && GetLastError() != 0 {
                return false;
            }
            let _ = SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
            GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & required != 0
        }
    }

    #[repr(C)]
    struct HighContrast {
        size: u32,
        flags: u32,
        scheme: *mut u16,
    }

    fn transparency_permitted() -> bool {
        let mut hc = HighContrast {
            size: std::mem::size_of::<HighContrast>() as u32,
            flags: 0,
            scheme: std::ptr::null_mut(),
        };
        // SPI_GETHIGHCONTRAST / HCF_HIGHCONTRASTON. Never change the system preference.
        unsafe {
            if SystemParametersInfoW(0x0042, hc.size, &mut hc as *mut _ as _, 0) != 0
                && hc.flags & 1 != 0
            {
                return false;
            }
        }
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
            .and_then(|key| key.get_value::<u32, _>("EnableTransparency"))
            .map(|enabled| enabled != 0)
            .unwrap_or(true)
    }

    pub fn animations_enabled() -> bool {
        let mut enabled = 1i32;
        // SPI_GETCLIENTAREAANIMATION, read on activation/interaction, never by a polling loop.
        unsafe {
            SystemParametersInfoW(0x1042, 0, &mut enabled as *mut _ as _, 0) == 0 || enabled != 0
        }
    }

    fn dark(mode: &str) -> bool {
        match mode {
            "dark" => true,
            "light" => false,
            _ => RegKey::predef(HKEY_CURRENT_USER)
                .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
                .and_then(|key| key.get_value::<u32, _>("AppsUseLightTheme"))
                .map(|light| light == 0)
                .unwrap_or(false),
        }
    }

    pub fn apply(material: Material, appearance: &str) -> bool {
        let hwnd = WINDOW.load(Ordering::Acquire) as *mut core::ffi::c_void;
        if hwnd.is_null() {
            return false;
        }
        let permitted = transparency_permitted() && unsafe { prepare_composition(hwnd) };
        let requested = material.backdrop(permitted);
        let dark = i32::from(dark(appearance));
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                &dark as *const _ as _,
                std::mem::size_of_val(&dark) as u32,
            );
            let hr = DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE as u32,
                &requested as *const _ as _,
                std::mem::size_of_val(&requested) as u32,
            );
            let active = requested != 1 && hr >= 0;
            let margin = if active { -1 } else { 0 };
            let margins = MARGINS {
                cxLeftWidth: margin,
                cxRightWidth: margin,
                cyTopHeight: margin,
                cyBottomHeight: margin,
            };
            if DwmExtendFrameIntoClientArea(hwnd, &margins) < 0 {
                let none = 1i32;
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_SYSTEMBACKDROP_TYPE as u32,
                    &none as *const _ as _,
                    std::mem::size_of_val(&none) as u32,
                );
                return false;
            }
            active
        }
    }

    pub fn install(window: &rune::WindowHandle, material: Material, appearance: &str) -> bool {
        window
            .with_raw_handle(|handle| {
                let RawWindowHandle::Win32(handle) = handle.as_raw() else {
                    return false;
                };
                WINDOW.store(handle.hwnd.get(), Ordering::Release);
                apply(material, appearance)
            })
            .unwrap_or(false)
    }
}

#[cfg(windows)]
pub use native::{animations_enabled, apply, install};

#[cfg(not(windows))]
pub fn animations_enabled() -> bool {
    true
}
#[cfg(not(windows))]
pub fn apply(_: Material, _: &str) -> bool {
    false
}
#[cfg(not(windows))]
pub fn install(_: &rune::WindowHandle, _: Material, _: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_preference_has_a_safe_explicit_opaque_mode() {
        for mode in [Material::Acrylic, Material::Mica, Material::Solid] {
            assert_eq!(Material::parse(mode.name()), mode);
            assert_eq!(mode.backdrop(false), 1);
        }
        assert_eq!(Material::Mica.backdrop(true), 2);
        assert_eq!(Material::Acrylic.backdrop(true), 3);
        assert_eq!(Material::Solid.backdrop(true), 1);
    }
}
