//! Taskbar embedding helpers, based on the sysmon (GuoJikun/sysmon) approach:
//! `SetParent` the overlay window into `Shell_TrayWnd`, then keep it anchored
//! next to `TrayNotifyWnd` via periodic repositioning.

use windows::core::w;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{ABM_GETTASKBARPOS, APPBARDATA, SHAppBarMessage};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetWindowRect, MoveWindow, SetParent,
};

/// Logical size of the overlay before DPI scaling (px @ 96 dpi).
pub const LOGICAL_WIDTH: f64 = 240.0;
pub const LOGICAL_HEIGHT: f64 = 28.0;

/// Reparent the given window into the taskbar. Returns true on success.
pub fn embed_into_taskbar(hwnd: HWND) -> bool {
    unsafe {
        let taskbar = match FindWindowW(w!("Shell_TrayWnd"), None) {
            Ok(t) if !t.is_invalid() => t,
            _ => return false,
        };
        let _ = SetParent(hwnd, Some(taskbar));
        true
    }
}

/// Position the overlay window next to `TrayNotifyWnd` (left for horizontal
/// taskbars, above for vertical ones). Call periodically so it survives taskbar
/// moves / explorer restarts / DPI changes.
pub fn reposition_in_taskbar(hwnd: HWND) {
    unsafe {
        let taskbar = match FindWindowW(w!("Shell_TrayWnd"), None) {
            Ok(t) if !t.is_invalid() => t,
            _ => return,
        };

        let dpi = GetDpiForWindow(hwnd);
        let scale = if dpi > 0 { dpi as f64 / 96.0 } else { 1.0 };
        let width = (LOGICAL_WIDTH * scale) as i32;
        let height = (LOGICAL_HEIGHT * scale) as i32;

        let mut abd = APPBARDATA::default();
        abd.cbSize = std::mem::size_of::<APPBARDATA>() as u32;
        if SHAppBarMessage(ABM_GETTASKBARPOS, &mut abd) == 0 {
            return;
        }
        let horizontal = (abd.rc.right - abd.rc.left) > (abd.rc.bottom - abd.rc.top);

        let tray = FindWindowExW(Some(taskbar), None, w!("TrayNotifyWnd"), None);
        let (tx, ty, th) = match tray {
            Ok(t) if !t.is_invalid() => {
                let mut r = RECT::default();
                if GetWindowRect(t, &mut r).is_ok() {
                    let mut lt = POINT { x: r.left, y: r.top };
                    let _ = ScreenToClient(taskbar, &mut lt);
                    (lt.x, lt.y, r.bottom - r.top)
                } else {
                    (abd.rc.right, abd.rc.top, abd.rc.bottom - abd.rc.top)
                }
            }
            _ => (abd.rc.right, abd.rc.top, abd.rc.bottom - abd.rc.top),
        };

        let (x, y) = if horizontal {
            (tx - width, ty + (th - height) / 2)
        } else {
            let mut bottom = POINT {
                x: abd.rc.right,
                y: abd.rc.bottom,
            };
            let _ = ScreenToClient(taskbar, &mut bottom);
            (bottom.x / 2 - width / 2, ty - height)
        };

        let _ = MoveWindow(hwnd, x, y, width, height, true);
    }
}
