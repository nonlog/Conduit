//! Client-drawn caption over the same single material as the sidebar and body.
//! Keep native resize/minimize/maximize styles so resize, system menu and snapping
//! remain Windows operations, not a borderless-window imitation.

pub const CAPTION_HEIGHT: f32 = 36.0;
pub const BUTTON_WIDTH: f32 = 46.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Client,
    Caption,
    Maximize,
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

fn hit_test(x: i32, y: i32, width: i32, height: i32, dpi: u32, maximized: bool) -> Hit {
    let scale = dpi.max(96) as f32 / 96.0;
    let edge = if maximized {
        0
    } else {
        (6.0 * scale).round() as i32
    };
    let (left, right, top, bottom) = (x < edge, x >= width - edge, y < edge, y >= height - edge);
    match (left, right, top, bottom) {
        (true, _, true, _) => return Hit::TopLeft,
        (_, true, true, _) => return Hit::TopRight,
        (true, _, _, true) => return Hit::BottomLeft,
        (_, true, _, true) => return Hit::BottomRight,
        (true, _, _, _) => return Hit::Left,
        (_, true, _, _) => return Hit::Right,
        (_, _, true, _) => return Hit::Top,
        (_, _, _, true) => return Hit::Bottom,
        _ => {}
    }
    let caption = (CAPTION_HEIGHT * scale).round() as i32;
    let button = (BUTTON_WIDTH * scale).round() as i32;
    if y >= 0 && y < caption {
        if x < width - button * 3 {
            return Hit::Caption;
        }
        if x >= width - button * 2 && x < width - button {
            return Hit::Maximize;
        }
    }
    Hit::Client
}

#[cfg(windows)]
mod native {
    use super::*;
    use rune::raw_window_handle::RawWindowHandle;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Dwm::DwmDefWindowProc;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, ScreenToClient,
    };
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    static WINDOW: AtomicIsize = AtomicIsize::new(0);
    const SUBCLASS: usize = 0x4344_4348;

    fn window() -> HWND {
        WINDOW.load(Ordering::Acquire) as HWND
    }

    unsafe fn work_area(hwnd: HWND) -> Option<MONITORINFO> {
        unsafe {
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            (GetMonitorInfoW(monitor, &mut info) != 0).then_some(info)
        }
    }

    unsafe extern "system" fn procedure(
        hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
        id: usize,
        _: usize,
    ) -> LRESULT {
        unsafe {
            match msg {
                WM_NCCALCSIZE => {
                    // lp begins with RECT for both NCCALCSIZE forms. A maximized
                    // client must stop at the work area, not cover the taskbar.
                    if lp != 0 && IsZoomed(hwnd) != 0 {
                        if let Some(info) = work_area(hwnd) {
                            *(lp as *mut RECT) = info.rcWork;
                        }
                    }
                    return 0;
                }
                WM_GETMINMAXINFO => {
                    let result = DefSubclassProc(hwnd, msg, wp, lp);
                    if lp != 0 {
                        if let Some(info) = work_area(hwnd) {
                            let bounds = &mut *(lp as *mut MINMAXINFO);
                            bounds.ptMaxPosition.x = info.rcWork.left - info.rcMonitor.left;
                            bounds.ptMaxPosition.y = info.rcWork.top - info.rcMonitor.top;
                            bounds.ptMaxSize.x = info.rcWork.right - info.rcWork.left;
                            bounds.ptMaxSize.y = info.rcWork.bottom - info.rcWork.top;
                        }
                    }
                    return result;
                }
                WM_NCHITTEST => {
                    let mut point = POINT {
                        x: (lp as u16 as i16) as i32,
                        y: ((lp >> 16) as u16 as i16) as i32,
                    };
                    let mut rect = RECT::default();
                    if ScreenToClient(hwnd, &mut point) != 0 && GetClientRect(hwnd, &mut rect) != 0
                    {
                        let hit = hit_test(
                            point.x,
                            point.y,
                            rect.right,
                            rect.bottom,
                            GetDpiForWindow(hwnd),
                            IsZoomed(hwnd) != 0,
                        );
                        return match hit {
                            Hit::Client => HTCLIENT,
                            Hit::Caption => HTCAPTION,
                            Hit::Maximize => HTMAXBUTTON,
                            Hit::Left => HTLEFT,
                            Hit::Right => HTRIGHT,
                            Hit::Top => HTTOP,
                            Hit::Bottom => HTBOTTOM,
                            Hit::TopLeft => HTTOPLEFT,
                            Hit::TopRight => HTTOPRIGHT,
                            Hit::BottomLeft => HTBOTTOMLEFT,
                            Hit::BottomRight => HTBOTTOMRIGHT,
                        } as LRESULT;
                    }
                }
                WM_NCLBUTTONDOWN if wp == HTMAXBUTTON as usize => return 0,
                WM_NCLBUTTONUP if wp == HTMAXBUTTON as usize => {
                    toggle_maximize();
                    return 0;
                }
                WM_NCDESTROY => {
                    RemoveWindowSubclass(hwnd, Some(procedure), id);
                    let _ = WINDOW.compare_exchange(
                        hwnd as isize,
                        0,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                }
                _ => {}
            }
            // DWM still owns native caption hover/Snap affordances, Alt+Space,
            // drag/double-click, and non-client activation behavior.
            let mut result = 0;
            if DwmDefWindowProc(hwnd, msg, wp, lp, &mut result) != 0 {
                return result;
            }
            DefSubclassProc(hwnd, msg, wp, lp)
        }
    }

    pub fn install(handle: &rune::WindowHandle) -> Result<(), String> {
        handle
            .with_raw_handle(|raw| {
                let RawWindowHandle::Win32(raw) = raw.as_raw() else {
                    return Err("Not a Windows window".into());
                };
                let hwnd = raw.hwnd.get() as HWND;
                if window() == hwnd {
                    return Ok(());
                }
                unsafe {
                    if SetWindowSubclass(hwnd, Some(procedure), SUBCLASS, 0) == 0 {
                        return Err("Could not install native caption".into());
                    }
                    // The client now paints the caption. Keeping WS_CAPTION makes
                    // DWM draw a second, faint set of native buttons over ours even
                    // though WM_NCCALCSIZE extends the client across that region.
                    // Retain THICKFRAME/SYSMENU/MINIMIZEBOX/MAXIMIZEBOX for native
                    // resize, taskbar, system-menu and Snap behavior.
                    let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
                    SetWindowLongPtrW(hwnd, GWL_STYLE, style & !(WS_CAPTION as isize));
                    WINDOW.store(hwnd as isize, Ordering::Release);
                    SetWindowPos(
                        hwnd,
                        std::ptr::null_mut(),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                    );
                }
                Ok(())
            })
            .unwrap_or_else(|_| Err("Native window is not ready".into()))
    }

    pub fn minimize() {
        let hwnd = window();
        if !hwnd.is_null() {
            unsafe {
                ShowWindow(hwnd, SW_MINIMIZE);
            }
        }
    }
    pub fn toggle_maximize() {
        let hwnd = window();
        if !hwnd.is_null() {
            unsafe {
                ShowWindow(
                    hwnd,
                    if IsZoomed(hwnd) != 0 {
                        SW_RESTORE
                    } else {
                        SW_MAXIMIZE
                    },
                );
            }
        }
    }
    pub fn close() {
        let hwnd = window();
        if !hwnd.is_null() {
            unsafe {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
        }
    }
}

#[cfg(windows)]
pub use native::{close, install, minimize, toggle_maximize};
#[cfg(not(windows))]
pub fn minimize() {}
#[cfg(not(windows))]
pub fn toggle_maximize() {}
#[cfg(not(windows))]
pub fn close() {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn caption_buttons_resize_edges_and_snap_zone_share_dpi_geometry() {
        for dpi in [96, 120, 144, 192] {
            let scale = dpi as f32 / 96.0;
            let px = |n: f32| (n * scale).round() as i32;
            let hit = |x: f32, y: f32| hit_test(px(x), px(y), px(1000.0), px(700.0), dpi, false);
            assert_eq!(hit(150.0, 20.0), Hit::Caption);
            assert_eq!(hit(1000.0 - BUTTON_WIDTH * 1.5, 20.0), Hit::Maximize);
            assert_eq!(hit(1000.0 - BUTTON_WIDTH * 2.5, 20.0), Hit::Client);
            assert_eq!(hit(1000.0 - BUTTON_WIDTH * 0.5, 20.0), Hit::Client);
            assert_eq!(hit(2.0, 2.0), Hit::TopLeft);
            assert_eq!(hit(998.0, 698.0), Hit::BottomRight);
            assert_eq!(hit(150.0, 100.0), Hit::Client);
            assert_eq!(
                hit_test(2, 2, px(1000.0), px(700.0), dpi, true),
                Hit::Caption
            );
        }
    }
}
