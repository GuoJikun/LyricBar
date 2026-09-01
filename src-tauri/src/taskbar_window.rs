use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tauri::async_runtime::spawn;
use tokio::time::interval;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{ABM_GETTASKBARPOS, APPBARDATA, SHAppBarMessage};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetParent, GetWindowRect, MoveWindow, SetParent,
    ShowWindow, SW_SHOWNA,
};

const LOGICAL_WIDTH: f64 = 240.0;
const LOGICAL_HEIGHT: f64 = 28.0;

static EMBEDDED: AtomicBool = AtomicBool::new(false);

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn create_lyric_window(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("[taskbar] creating lyric window");

    let _win = WebviewWindowBuilder::new(
        app,
        "lyric",
        WebviewUrl::App("lyric.html".into()),
    )
    .title("LyricBar")
    .inner_size(LOGICAL_WIDTH, LOGICAL_HEIGHT)
    .decorations(false)
    .skip_taskbar(true)
    .resizable(false)
    .transparent(true)
    .shadow(false)
    .visible(false)
    .build()?;

    log::info!("[taskbar] window created, waiting for frontend to invoke embed");
    Ok(())
}

#[tauri::command]
pub fn embed_lyric_window(app: tauri::AppHandle) {
    if EMBEDDED.load(Ordering::Relaxed) {
        log::info!("[taskbar] embed: already embedded, skipping");
        return;
    }

    let Some(win) = app.get_webview_window("lyric") else {
        log::error!("[taskbar] embed: lyric window not found");
        return;
    };

    let Ok(hwnd) = win.hwnd() else {
        log::error!("[taskbar] embed: failed to get HWND");
        return;
    };

    unsafe {
        let h_taskbar = match FindWindowW(PCWSTR::from_raw(w("Shell_TrayWnd").as_ptr()), None) {
            Ok(h) if !h.is_invalid() => h,
            _ => {
                log::error!("[taskbar] embed: Shell_TrayWnd not found!");
                return;
            }
        };

        let result = SetParent(hwnd, Some(h_taskbar));
        log::info!("[taskbar] embed: SetParent returned {:?}", result);

        if let Ok(parent) = GetParent(hwnd) {
            if parent == h_taskbar {
                log::info!("[taskbar] embed: verified - parent is Shell_TrayWnd");
            } else {
                log::error!(
                    "[taskbar] embed: WARNING - parent mismatch! expected {:?}, got {:?}",
                    h_taskbar, parent
                );
            }
        }

        EMBEDDED.store(true, Ordering::Relaxed);
        let _ = ShowWindow(hwnd, SW_SHOWNA);
        log::info!("[taskbar] embed: SUCCESS");
    }

    start_reposition_timer(app);
}

pub fn reposition(hwnd: HWND) {
    unsafe {
        let h_taskbar = match FindWindowW(PCWSTR::from_raw(w("Shell_TrayWnd").as_ptr()), None) {
            Ok(h) if !h.is_invalid() => h,
            _ => return,
        };

        let dpi = GetDpiForWindow(hwnd);
        let scale = if dpi > 0 { dpi as f64 / 96.0 } else { 1.0 };
        let win_width = (LOGICAL_WIDTH * scale) as i32;
        let win_height = (LOGICAL_HEIGHT * scale) as i32;

        let mut abd = APPBARDATA::default();
        abd.cbSize = std::mem::size_of::<APPBARDATA>() as u32;
        if SHAppBarMessage(ABM_GETTASKBARPOS, &mut abd) == 0 {
            return;
        }
        let is_horizontal = (abd.rc.right - abd.rc.left) > (abd.rc.bottom - abd.rc.top);

        let h_tray = FindWindowExW(
            Some(h_taskbar), None,
            PCWSTR::from_raw(w("TrayNotifyWnd").as_ptr()), None,
        )
        .ok()
        .filter(|h| !h.is_invalid());

        let (tray_x, tray_y, tray_h) = match h_tray {
            Some(h) => {
                let mut r = RECT::default();
                if GetWindowRect(h, &mut r).is_ok() {
                    let mut lt = POINT { x: r.left, y: r.top };
                    let mut rb = POINT { x: r.right, y: r.bottom };
                    let _ = ScreenToClient(h_taskbar, &mut lt);
                    let _ = ScreenToClient(h_taskbar, &mut rb);
                    (lt.x, lt.y, rb.y - lt.y)
                } else {
                    let mut pt = POINT { x: abd.rc.right, y: abd.rc.top };
                    let _ = ScreenToClient(h_taskbar, &mut pt);
                    (pt.x, pt.y, abd.rc.bottom - abd.rc.top)
                }
            }
            None => {
                let mut pt = POINT { x: abd.rc.right, y: abd.rc.top };
                let _ = ScreenToClient(h_taskbar, &mut pt);
                (pt.x, pt.y, abd.rc.bottom - abd.rc.top)
            }
        };

        let (x, y) = if is_horizontal {
            (tray_x - win_width, tray_y + (tray_h - win_height) / 2)
        } else {
            let tb_w_client = {
                let mut pt = POINT { x: abd.rc.right, y: abd.rc.bottom };
                let _ = ScreenToClient(h_taskbar, &mut pt);
                pt.x
            };
            (tb_w_client / 2 - win_width / 2, tray_y - win_height)
        };

        log::debug!(
            "[taskbar] reposition: pos({}, {}), size {}x{}",
            x, y, win_width, win_height
        );
        let _ = MoveWindow(hwnd, x, y, win_width, win_height, false);
    }
}

fn start_reposition_timer(app_handle: tauri::AppHandle) {
    spawn(async move {
        let mut tick = interval(Duration::from_millis(500));
        loop {
            tick.tick().await;
            if let Some(win) = app_handle.get_webview_window("lyric") {
                if let Ok(hwnd) = win.hwnd() {
                    reposition(hwnd);
                }
            }
        }
    });
}
