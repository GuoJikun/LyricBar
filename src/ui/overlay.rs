//! 歌词悬浮窗口，复刻 Tauri 的 DwmEnableBlurBehindWindow 方案，
//! 创建 WS_POPUP 顶层窗口后 SetParent 嵌入 Shell_TrayWnd。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateRectRgn, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint,
    FillRect, InvalidateRect, ScreenToClient, SelectObject, SetBkMode, SetTextColor,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_QUALITY, DEFAULT_PITCH,
    DT_CENTER, DT_END_ELLIPSIS, DT_SINGLELINE, DT_VCENTER, DT_BOTTOM, DT_TOP, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::Graphics::Dwm::{DwmEnableBlurBehindWindow, DWM_BLURBEHIND, DWM_BB_ENABLE, DWM_BB_BLURREGION};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{ABM_GETTASKBARPOS, APPBARDATA, SHAppBarMessage};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, FindWindowExW, FindWindowW,
    GetMessageW, GetClientRect, GetWindowLongPtrW, GetWindowRect, MoveWindow, RegisterClassW,
    SetTimer, SetWindowLongPtrW, SetParent, ShowWindow, TranslateMessage,
    GWLP_USERDATA, MSG, SW_SHOWNA, WNDCLASSW, WS_EX_NOACTIVATE,
    WS_POPUP, WS_VISIBLE, WS_CLIPSIBLINGS, WM_DESTROY, WM_PAINT, WM_TIMER,
};

/// 背景色（浅灰，用于调试确认窗口可见）
const BG_COLOR: u32 = 0x00404040;
const CLASS_NAME: windows::core::PCWSTR = w!("LyricBarOverlay");
pub const LOGICAL_WIDTH: f64 = 240.0;
pub const LOGICAL_HEIGHT: f64 = 28.0;

pub struct OverlayState {
    pub text: String,
    pub subtext: String,
}

impl Default for OverlayState {
    fn default() -> Self {
        Self { text: String::new(), subtext: String::new() }
    }
}

pub struct Overlay {
    hwnd: Arc<Mutex<Option<usize>>>,
    state: Arc<Mutex<OverlayState>>,
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

/// 复刻 Tauri tao/window.rs:1284-1297 的 DwmEnableBlurBehindWindow 调用。
/// 使用空 region 启用 DWM 合成，使窗口内容能被 DWM 正确渲染。
unsafe fn enable_dwm_blur(hwnd: HWND) {
    let region = CreateRectRgn(0, 0, -1, -1); // 空 region
    let bb = DWM_BLURBEHIND {
        dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
        fEnable: windows::core::BOOL(1),
        hRgnBlur: region,
        fTransitionOnMaximized: windows::core::BOOL(0),
    };
    let _ = DwmEnableBlurBehindWindow(hwnd, &bb);
    let _ = DeleteObject(region.into());
    log::debug!("overlay: DwmEnableBlurBehindWindow 已调用");
}

/// 在任务栏内定位窗口（Shell_TrayWnd 客户区坐标）。
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
            let mut bottom = POINT { x: abd.rc.right, y: abd.rc.bottom };
            let _ = ScreenToClient(taskbar, &mut bottom);
            (bottom.x / 2 - width / 2, ty - height)
        };

        log::debug!(
            "[taskbar] 重定位: 位置({}, {}), 尺寸 {}x{}, 托盘=({},{} _x{})",
            x, y, width, height, tx, ty, th
        );
        let _ = MoveWindow(hwnd, x, y, width, height, true);
    }
}

/// 使用 GDI WM_PAINT 绘制窗口内容（标准 DWM 合成路径）。
fn paint_window(hwnd: HWND, state: &OverlayState) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        if hdc.is_invalid() {
            return;
        }

        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;

        // 绘制灰色背景
        let brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(BG_COLOR));
        let _ = FillRect(hdc, &rect, brush);
        let _ = DeleteObject(brush.into());

        // 绘制文字
        if !state.text.is_empty() || !state.subtext.is_empty() {
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, windows::Win32::Foundation::COLORREF(rgb(255, 255, 255)));

            let face = to_wide("Microsoft YaHei");
            let font = CreateFontW(
                16, 0, 0, 0, 700, 0, 0, 0,
                DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
                DEFAULT_QUALITY, DEFAULT_PITCH.0 as u32,
                windows::core::PCWSTR(face.as_ptr()),
            );
            let old_font = SelectObject(hdc, font.into());

            if state.subtext.is_empty() {
                let mut wide = to_wide(&state.text);
                let mut draw_rect = RECT { left: 0, top: 0, right: w, bottom: h };
                    DrawTextW(
                    hdc, &mut wide, &mut draw_rect,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
                );
            } else {
                let mid = h / 2;
                let mut wtext = to_wide(&state.text);
                let mut top_rect = RECT { left: 0, top: 0, right: w, bottom: mid };
                DrawTextW(
                    hdc, &mut wtext, &mut top_rect,
                    DT_CENTER | DT_BOTTOM | DT_SINGLELINE | DT_END_ELLIPSIS,
                );
                let mut wsub = to_wide(&state.subtext);
                let mut bot_rect = RECT { left: 0, top: mid, right: w, bottom: h };
                DrawTextW(
                    hdc, &mut wsub, &mut bot_rect,
                    DT_CENTER | DT_TOP | DT_SINGLELINE | DT_END_ELLIPSIS,
                );
            }

            let _ = SelectObject(hdc, old_font);
            let _ = DeleteObject(font.into());
        }

        let _ = EndPaint(hwnd, &ps);
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let label = match msg {
        WM_PAINT => Some("WM_PAINT"),
        WM_TIMER => Some("WM_TIMER"),
        WM_DESTROY => Some("WM_DESTROY"),
        _ => None,
    };
    if let Some(l) = label {
        log::debug!("wndproc 收到消息: {l} (0x{msg:04X})");
    }
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match msg {
        WM_PAINT => {
            let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize };
            if raw != 0 {
                let state_ref = unsafe { &*(raw as *const Mutex<OverlayState>) };
                let guard = state_ref.lock().unwrap();
                paint_window(hwnd, &guard);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            unsafe {
                reposition_in_taskbar(hwnd);
                // Timer 时也重绘（文字可能变化）
                let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize;
                if raw != 0 {
                    let state_ref = &*(raw as *const Mutex<OverlayState>);
                    let guard = state_ref.lock().unwrap();
                    paint_window(hwnd, &guard);
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }));
    match r {
        Ok(v) => v,
        Err(e) => {
            log::error!("wndproc panic @msg={label:?}: {e:?}");
            LRESULT(0)
        }
    }
}

impl Overlay {
    pub fn new() -> anyhow::Result<Self> {
        let state = Arc::new(Mutex::new(OverlayState::default()));
        let hwnd_slot: Arc<Mutex<Option<usize>>> = Arc::new(Mutex::new(None));

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

        let state_thread = state.clone();
        let hwnd_slot_thread = hwnd_slot.clone();
        std::thread::spawn(move || unsafe {
            let hinst = GetModuleHandleW(None).unwrap_or_default();

            // 查找 Shell_TrayWnd
            let taskbar = match FindWindowW(w!("Shell_TrayWnd"), None) {
                Ok(t) if !t.is_invalid() => t,
                _ => {
                    log::error!("overlay: 未找到 Shell_TrayWnd");
                    return;
                }
            };
            log::debug!("overlay: 找到 Shell_TrayWnd {:?}", taskbar);

            // 复刻 Tauri 的窗口创建方式：
            // 1. WS_POPUP（无标题栏/边框）+ WS_VISIBLE + WS_CLIPSIBLINGS
            // 2. 无 WS_EX_LAYERED（用 DwmEnableBlurBehindWindow 代替）
            // 3. 仅 WS_EX_NOACTIVATE 防止抢焦点
            let hwnd = match CreateWindowExW(
                WS_EX_NOACTIVATE,
                CLASS_NAME,
                windows::core::PCWSTR::null(),
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
            log::debug!("overlay: WS_POPUP 窗口创建成功 hwnd={:?}", hwnd);

            // 复刻 Tauri 的 DwmEnableBlurBehindWindow（启用 DWM 合成）
            enable_dwm_blur(hwnd);

            // SetParent 嵌入任务栏
            let prev = SetParent(hwnd, Some(taskbar));
            log::debug!("overlay: SetParent 返回 {:?}", prev);

            // 复刻 sysmon：SetParent 后用 SW_SHOWNA 显示（不激活）
            let _ = ShowWindow(hwnd, SW_SHOWNA);

            // 存储 state 引用
            let leaked = Arc::into_raw(state_thread.clone());
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, leaked as isize);

            // 初始定位 + 重绘
            reposition_in_taskbar(hwnd);
            let state_ref = &*leaked;
            let guard = state_ref.lock().unwrap();
            paint_window(hwnd, &guard);
            drop(guard);
            log::debug!("overlay: 初始定位 + 绘制完成");

            // 定时器：周期性重定位 + 重绘
            SetTimer(Some(hwnd), 1, 200, None);

            *hwnd_slot_thread.lock().unwrap() = Some(hwnd.0 as usize);

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                let _ = DispatchMessageW(&msg);
            }
            let _ = Arc::from_raw(leaked);
        });

        Ok(Self { hwnd: hwnd_slot, state })
    }

    pub fn set_text(&self, text: &str, subtext: &str) {
        {
            let mut s = self.state.lock().unwrap();
            s.text = text.to_string();
            s.subtext = subtext.to_string();
        }
        if let Some(v) = *self.hwnd.lock().unwrap() {
            let hwnd = HWND(v as *mut std::ffi::c_void);
            if !hwnd.is_invalid() {
                log::debug!("set_text: 更新内容主={:?} 副={:?}", text, subtext);
                // 触发重绘
                unsafe {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
        }
    }
}

#[allow(dead_code)]
fn _duration_marker(_: Duration) {}
