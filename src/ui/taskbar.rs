//! 任务栏嵌入辅助函数，基于 sysmon（GuoJikun/sysmon）的方案：
//! 将悬浮窗通过 `SetParent` 挂到 `Shell_TrayWnd`，再借助周期性重定位
//! 使其始终紧贴 `TrayNotifyWnd`。

use windows::core::w;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{ABM_GETTASKBARPOS, APPBARDATA, SHAppBarMessage};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetParent, GetWindowRect, MoveWindow, SetParent,
};

/// 悬浮窗在 DPI 缩放前的逻辑尺寸（基于 96 dpi 的像素）。
pub const LOGICAL_WIDTH: f64 = 240.0;
pub const LOGICAL_HEIGHT: f64 = 28.0;

/// 将给定窗口重父化到任务栏中。成功返回 true。
pub fn embed_into_taskbar(hwnd: HWND) -> bool {
    unsafe {
        let taskbar = match FindWindowW(w!("Shell_TrayWnd"), None) {
            Ok(t) if !t.is_invalid() => t,
            _ => {
                log::error!("[taskbar] 嵌入: 未找到 Shell_TrayWnd!");
                return false;
            }
        };
        let result = SetParent(hwnd, Some(taskbar));
        log::info!("[taskbar] 嵌入: SetParent 返回 {:?}", result);

        if let Ok(parent) = GetParent(hwnd) {
            if parent == taskbar {
                log::info!("[taskbar] 嵌入: 验证通过 - 父窗口是 Shell_TrayWnd");
            } else {
                log::error!(
                    "[taskbar] 嵌入: 警告 - 父窗口不匹配! 期望 {:?}, 实际 {:?}",
                    taskbar, parent
                );
            }
        } else {
            log::error!("[taskbar] 嵌入: GetParent 失败");
        }
        true
    }
}

/// 将悬浮窗定位到 `TrayNotifyWnd` 旁（横向任务栏时在左侧，纵向时在上方）。
/// 需周期性调用，以在任务栏移动、资源管理器重启、DPI 变化时仍保持正确位置。
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
                    let mut rb = POINT { x: r.right, y: r.bottom };
                    let _ = ScreenToClient(taskbar, &mut lt);
                    let _ = ScreenToClient(taskbar, &mut rb);
                    (lt.x, lt.y, rb.y - lt.y)
                } else {
                    let mut pt = POINT { x: abd.rc.right, y: abd.rc.top };
                    let _ = ScreenToClient(taskbar, &mut pt);
                    (pt.x, pt.y, abd.rc.bottom - abd.rc.top)
                }
            }
            _ => {
                let mut pt = POINT { x: abd.rc.right, y: abd.rc.top };
                let _ = ScreenToClient(taskbar, &mut pt);
                (pt.x, pt.y, abd.rc.bottom - abd.rc.top)
            }
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

        log::debug!(
            "[taskbar] 重定位: 位置({}, {}), 尺寸 {}x{}, 托盘=({},{} {}x{})",
            x, y, width, height, tx, ty, 0, th
        );
        let _ = MoveWindow(hwnd, x, y, width, height, true);
    }
}
