//! 歌词悬浮窗口，使用 Tauri webview 渲染歌词 HTML，
//! 通过 SetParent(Shell_TrayWnd) 嵌入任务栏。

use std::sync::{mpsc, Arc, Mutex};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const LOGICAL_WIDTH: f64 = 240.0;
pub const LOGICAL_HEIGHT: f64 = 28.0;

struct ThreadState {
    rx: mpsc::Receiver<String>,
    #[allow(dead_code)]
    hwnd: HWND,
}

pub struct Overlay {
    hwnd: Arc<Mutex<Option<usize>>>,
    tx: mpsc::Sender<String>,
}

const CLASS_NAME: PCWSTR = w!("LyricBarOverlay");

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER => {
                reposition_in_taskbar(hwnd);
                let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ThreadState;
                if !raw.is_null() {
                    let state = &mut *raw;
                    while let Ok(_html) = state.rx.try_recv() {
                        log::debug!("overlay: 收到更新");
                    }
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn reposition_in_taskbar(hwnd: HWND) {
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
        let (tx_pos, ty, th) = match tray {
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
            (tx_pos - width, ty + (th - height) / 2)
        } else {
            let mut bottom = POINT { x: abd.rc.right, y: abd.rc.bottom };
            let _ = ScreenToClient(taskbar, &mut bottom);
            (bottom.x / 2 - width / 2, ty - height)
        };

        log::debug!(
            "[taskbar] 重定位: 位置({}, {}), 尺寸 {}x{}, 托盘=({},{} _x{})",
            x, y, width, height, tx_pos, ty, th
        );
        let _ = MoveWindow(hwnd, x, y, width, height, true);
    }
}

impl Overlay {
    pub fn new() -> anyhow::Result<Self> {
        let hwnd_slot: Arc<Mutex<Option<usize>>> = Arc::new(Mutex::new(None));
        let (tx, rx) = mpsc::channel::<String>();

        unsafe {
            let hinst = GetModuleHandleW(None).unwrap_or_default();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinst.into(),
                lpszClassName: CLASS_NAME,
                ..Default::default()
            };
            RegisterClassW(&wc);
        }

        let hwnd_slot_thread = hwnd_slot.clone();
        std::thread::spawn(move || unsafe {
            let hinst = GetModuleHandleW(None).unwrap_or_default();

            let taskbar = match FindWindowW(w!("Shell_TrayWnd"), None) {
                Ok(t) if !t.is_invalid() => t,
                _ => {
                    log::error!("overlay: 未找到 Shell_TrayWnd");
                    return;
                }
            };
            log::debug!("overlay: 找到 Shell_TrayWnd {:?}", taskbar);

            let hwnd = match CreateWindowExW(
                WS_EX_NOACTIVATE,
                CLASS_NAME,
                PCWSTR::null(),
                WS_POPUP | WS_VISIBLE | WS_CLIPSIBLINGS,
                0, 0, 240, 28,
                None,
                None,
                Some(hinst.into()),
                None,
            ) {
                Ok(h) => h,
                Err(e) => {
                    log::error!("overlay: CreateWindowExW 失败: {e}");
                    return;
                }
            };
            if hwnd.is_invalid() {
                log::error!("overlay: 窗口句柄无效");
                return;
            }
            log::debug!("overlay: 窗口创建成功 hwnd={:?}", hwnd);

            {
                let margins = windows::Win32::UI::Controls::MARGINS {
                    cxLeftWidth: -1,
                    cxRightWidth: -1,
                    cyTopHeight: -1,
                    cyBottomHeight: -1,
                };
                let _ = windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea(hwnd, &margins);
            }

            let prev = SetParent(hwnd, Some(taskbar));
            log::debug!("overlay: SetParent 返回 {:?}", prev);
            let _ = ShowWindow(hwnd, SW_SHOWNA);

            let mut thread_state = ThreadState { rx, hwnd };
            let state_ptr: *mut ThreadState = &mut thread_state;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);

            reposition_in_taskbar(hwnd);
            SetTimer(Some(hwnd), 1, 500, None);

            *hwnd_slot_thread.lock().unwrap() = Some(hwnd.0 as usize);
            log::debug!("overlay: 进入消息循环");

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                let _ = DispatchMessageW(&msg);
            }

            drop(thread_state);
        });

        Ok(Self { hwnd: hwnd_slot, tx })
    }

    pub fn set_text(&self, _text: &str, _subtext: &str) {
        log::debug!("set_text: 主={:?} 副={:?}", _text, _subtext);
    }
}
