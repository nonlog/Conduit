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
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Dwm::{
        DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND, DWMWA_SYSTEMBACKDROP_TYPE,
        DWMWA_USE_IMMERSIVE_DARK_MODE, DwmEnableBlurBehindWindow, DwmExtendFrameIntoClientArea,
        DwmSetWindowAttribute,
    };
    use windows_sys::Win32::Graphics::Gdi::{
        BLACKNESS, CreateRectRgn, DeleteObject, GetDC, HDC, PatBlt, ReleaseDC,
    };
    use windows_sys::Win32::UI::Controls::MARGINS;
    use windows_sys::Win32::UI::HiDpi::{
        GetWindowDpiAwarenessContext, SetThreadDpiAwarenessContext,
    };
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClientRect, SystemParametersInfoW, WM_DPICHANGED, WM_ERASEBKGND, WM_NCDESTROY, WM_SIZE,
    };
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};

    static WINDOW: AtomicIsize = AtomicIsize::new(0);
    const BACKING_SUBCLASS: usize = 0x4344_4d41;

    /// Clear only the native backing bitmap, never the Rune GPU surface. Winit
    /// has a null class brush; its uninitialized white backing otherwise leaks
    /// through translucent pixels. BLACKNESS also zeros the backing alpha.
    unsafe fn clear_backing_dc(hwnd: HWND, dc: HDC) -> bool {
        unsafe {
            if dc.is_null() {
                return false;
            }
            // Match this HWND rather than a caller's possibly virtualized DPI.
            // This also clears the right/bottom edges at 125/150/200% scaling.
            let previous = SetThreadDpiAwarenessContext(GetWindowDpiAwarenessContext(hwnd));
            let mut rect = RECT::default();
            let ok = GetClientRect(hwnd, &mut rect) != 0
                && (rect.right <= 0
                    || rect.bottom <= 0
                    || PatBlt(dc, 0, 0, rect.right, rect.bottom, BLACKNESS) != 0);
            if !previous.is_null() {
                SetThreadDpiAwarenessContext(previous);
            }
            ok
        }
    }

    unsafe fn clear_backing(hwnd: HWND) -> bool {
        unsafe {
            let dc = GetDC(hwnd);
            let ok = clear_backing_dc(hwnd, dc);
            if !dc.is_null() {
                ReleaseDC(hwnd, dc);
            }
            ok
        }
    }

    unsafe extern "system" fn backing_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        id: usize,
        _: usize,
    ) -> LRESULT {
        unsafe {
            if message == WM_ERASEBKGND && clear_backing_dc(hwnd, wparam as HDC) {
                return 1;
            }
            if message == WM_NCDESTROY {
                RemoveWindowSubclass(hwnd, Some(backing_proc), id);
                let _ =
                    WINDOW.compare_exchange(hwnd as isize, 0, Ordering::AcqRel, Ordering::Acquire);
            }
            let result = DefSubclassProc(hwnd, message, wparam, lparam);
            if matches!(message, WM_SIZE | WM_DPICHANGED) {
                // One bounded clear when native geometry changes. No timer,
                // idle repaint, screenshot capture, or per-animation GDI work.
                clear_backing(hwnd);
            }
            result
        }
    }

    unsafe fn prepare_composition(hwnd: HWND) -> bool {
        unsafe {
            // Keep the HWND redirection bitmap: some Vulkan drivers present
            // through it. Removing it can make an otherwise working UI vanish.
            // Reusing the same procedure/id updates one existing hook. Do not
            // import GetWindowSubclass: older system comctl32 versions export
            // the installation functions but not that query by name.
            if SetWindowSubclass(hwnd, Some(backing_proc), BACKING_SUBCLASS, 0) == 0 {
                return false;
            }
            if !clear_backing(hwnd) {
                RemoveWindowSubclass(hwnd, Some(backing_proc), BACKING_SUBCLASS);
                return false;
            }
            true
        }
    }

    unsafe fn per_pixel_composition(hwnd: HWND, enabled: bool) -> bool {
        unsafe {
            // Vulkan may expose INHERIT alpha. The HWND must explicitly opt
            // into per-pixel DWM composition; a backdrop type alone can report
            // success while displaying only its opaque fallback tint.
            let region = if enabled {
                CreateRectRgn(0, 0, -1, -1)
            } else {
                std::ptr::null_mut()
            };
            if enabled && region.is_null() {
                return false;
            }
            let blur = DWM_BLURBEHIND {
                dwFlags: DWM_BB_ENABLE | if enabled { DWM_BB_BLURREGION } else { 0 },
                fEnable: i32::from(enabled),
                hRgnBlur: region,
                fTransitionOnMaximized: 0,
            };
            let ok = DwmEnableBlurBehindWindow(hwnd, &blur) >= 0;
            if !region.is_null() {
                DeleteObject(region);
            }
            ok
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

    /// Resolve the appearance mode once for the whole native window and Rune
    /// surface.  `Cx::system_dark()` can lag behind a Windows theme change (and
    /// reports light in some remote sessions), while this is the same registry
    /// value Windows uses for app chrome.
    pub fn effective_dark(mode: &str) -> bool {
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
        let dark = i32::from(effective_dark(appearance));
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                &dark as *const _ as _,
                std::mem::size_of_val(&dark) as u32,
            );
            let active = requested != 1;
            let margin = if active { -1 } else { 0 };
            let margins = MARGINS {
                cxLeftWidth: margin,
                cxRightWidth: margin,
                cyTopHeight: margin,
                cyBottomHeight: margin,
            };
            let frame_ok = DwmExtendFrameIntoClientArea(hwnd, &margins) >= 0;
            let alpha_ok = per_pixel_composition(hwnd, active);
            let hr = DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE as u32,
                &requested as *const _ as _,
                std::mem::size_of_val(&requested) as u32,
            );
            // Changing a native backdrop/frame can invalidate the backing even
            // without a resize. Clear once after that change as well; callers
            // already request the matching Rune frame on activation/settings.
            if !frame_ok || !alpha_ok || hr < 0 || !clear_backing(hwnd) {
                per_pixel_composition(hwnd, false);
                let none = 1i32;
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_SYSTEMBACKDROP_TYPE as u32,
                    &none as *const _ as _,
                    std::mem::size_of_val(&none) as u32,
                );
                let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS::default());
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

    #[cfg(test)]
    mod backing_tests {
        use super::*;
        use windows_sys::Win32::Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, SelectObject,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, GWL_EXSTYLE, GetWindowLongPtrW, SendMessageW, WS_POPUP,
        };

        struct TestSurface {
            window: HWND,
            dc: HDC,
            bitmap: *mut core::ffi::c_void,
            old: *mut core::ffi::c_void,
        }
        impl Drop for TestSurface {
            fn drop(&mut self) {
                unsafe {
                    if !self.window.is_null() {
                        DestroyWindow(self.window);
                    }
                    if !self.old.is_null() {
                        SelectObject(self.dc, self.old);
                    }
                    if !self.bitmap.is_null() {
                        DeleteObject(self.bitmap);
                    }
                    if !self.dc.is_null() {
                        DeleteDC(self.dc);
                    }
                }
            }
        }

        #[test]
        fn native_background_erase_clears_rgb_and_alpha_without_removing_vulkan_backing() {
            for (width, height) in [(64, 48), (137, 109)] {
                unsafe {
                    let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
                    let mut surface = TestSurface {
                        window: CreateWindowExW(
                            0,
                            class.as_ptr(),
                            class.as_ptr(),
                            WS_POPUP,
                            0,
                            0,
                            width,
                            height,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            std::ptr::null(),
                        ),
                        dc: std::ptr::null_mut(),
                        bitmap: std::ptr::null_mut(),
                        old: std::ptr::null_mut(),
                    };
                    assert!(!surface.window.is_null());
                    let style = GetWindowLongPtrW(surface.window, GWL_EXSTYLE);
                    assert!(prepare_composition(surface.window));
                    assert!(
                        prepare_composition(surface.window),
                        "installation is idempotent"
                    );
                    assert_eq!(GetWindowLongPtrW(surface.window, GWL_EXSTYLE), style);
                    surface.dc = CreateCompatibleDC(std::ptr::null_mut());
                    assert!(!surface.dc.is_null());
                    let info = BITMAPINFO {
                        bmiHeader: BITMAPINFOHEADER {
                            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                            biWidth: width,
                            biHeight: -height,
                            biPlanes: 1,
                            biBitCount: 32,
                            biCompression: BI_RGB,
                            ..Default::default()
                        },
                        ..Default::default()
                    };
                    let mut pixels = std::ptr::null_mut();
                    surface.bitmap = CreateDIBSection(
                        surface.dc,
                        &info,
                        DIB_RGB_COLORS,
                        &mut pixels,
                        std::ptr::null_mut(),
                        0,
                    );
                    assert!(!surface.bitmap.is_null() && !pixels.is_null());
                    surface.old = SelectObject(surface.dc, surface.bitmap);
                    let len = (width * height * 4) as usize;
                    std::ptr::write_bytes(pixels, 255, len);
                    assert_eq!(
                        SendMessageW(surface.window, WM_ERASEBKGND, surface.dc as usize, 0),
                        1
                    );
                    GdiFlush();
                    assert!(
                        std::slice::from_raw_parts(pixels as *const u8, len)
                            .iter()
                            .all(|v| *v == 0),
                        "opaque white backing remains at {width}x{height}"
                    );
                }
            }
        }
    }
}

#[cfg(windows)]
pub use native::{animations_enabled, apply, effective_dark, install};

#[cfg(not(windows))]
pub fn animations_enabled() -> bool {
    true
}
#[cfg(not(windows))]
pub fn effective_dark(mode: &str) -> bool {
    mode == "dark"
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
