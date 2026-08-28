//! 透明、可点击穿透的歌词窗口，创建时直接以 WS_CHILD 嵌入 Shell_TrayWnd，
//! 使用 UpdateLayeredWindow 渲染，确保 DWM 合成管线能正确绘制。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject,
    EndPaint, GetDC, ReleaseDC, ScreenToClient, SelectObject, SetBkMode, SetTextColor,
    BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
    DEFAULT_QUALITY, DEFAULT_PITCH, DIB_RGB_COLORS, DT_CENTER, DT_END_ELLIPSIS, DT_SINGLELINE,
    DT_VCENTER, OUT_DEFAULT_PRECIS, PAINTSTRUCT, RGBQUAD, TRANSPARENT, AC_SRC_ALPHA, AC_SRC_OVER,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{ABM_GETTASKBARPOS, APPBARDATA, SHAppBarMessage};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, FindWindowExW, FindWindowW,
    GetMessageW, GetWindowLongPtrW, GetWindowRect, MoveWindow,
    RegisterClassW, SetTimer, SetWindowLongPtrW, SetParent, TranslateMessage, UpdateLayeredWindow,
    GWLP_USERDATA, MSG, UPDATE_LAYERED_WINDOW_FLAGS, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TRANSPARENT, WS_POPUP, WM_DESTROY, WM_PAINT, WM_TIMER,
};

/// 用于调试的浅灰色背景（0x00404040 = RGB(64,64,64)）。
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

        // 获取 TrayNotifyWnd 在 Shell_TrayWnd 客户区中的位置
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

/// 使用 UpdateLayeredWindow 渲染窗口内容到 DWM 合成表面。
unsafe fn update_layered(hwnd: HWND, state: &OverlayState) {
    let hdc_screen = GetDC(None);
    if hdc_screen.is_invalid() { return; }
    let hdc_mem = CreateCompatibleDC(Some(hdc_screen));
    if hdc_mem.is_invalid() {
        let _ = ReleaseDC(None, hdc_screen);
        return;
    }

    let mut rect = RECT::default();
    let _ = GetWindowRect(hwnd, &mut rect);
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    if w <= 0 || h <= 0 {
        let _ = DeleteDC(hdc_mem);
        let _ = ReleaseDC(None, hdc_screen);
        return;
    }

    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0,
            ..Default::default()
        },
        bmiColors: [RGBQUAD::default(); 1],
    };
    let mut pixels: *mut std::ffi::c_void = std::ptr::null_mut();
    let hbitmap = match CreateDIBSection(
        Some(hdc_mem),
        &bmi,
        DIB_RGB_COLORS,
        &mut pixels,
        None,
        0,
    ) {
        Ok(h) if !h.is_invalid() => h,
        _ => {
            let _ = DeleteDC(hdc_mem);
            let _ = ReleaseDC(None, hdc_screen);
            return;
        }
    };
    let old_bmp = SelectObject(hdc_mem, hbitmap.into());

    // 绘制灰色背景
    let pixel_slice = std::slice::from_raw_parts_mut(pixels as *mut u32, (w * h) as usize);
    pixel_slice.fill(BG_COLOR);

    // 绘制文字
    if !state.text.is_empty() || !state.subtext.is_empty() {
        SetBkMode(hdc_mem, TRANSPARENT);
        SetTextColor(hdc_mem, windows::Win32::Foundation::COLORREF(rgb(255, 255, 255)));

        let face = to_wide("Microsoft YaHei");
        let font = CreateFontW(
            16, 0, 0, 0, 700, 0, 0, 0,
            DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
            DEFAULT_QUALITY, DEFAULT_PITCH.0 as u32,
            windows::core::PCWSTR(face.as_ptr()),
        );
        let old_font = SelectObject(hdc_mem, font.into());

        if state.subtext.is_empty() {
            let mut wide = to_wide(&state.text);
            let mut draw_rect = RECT { left: 0, top: 0, right: w, bottom: h };
            windows::Win32::Graphics::Gdi::DrawTextW(
                hdc_mem, &mut wide, &mut draw_rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
        } else {
            let mid = h / 2;
            let mut wtext = to_wide(&state.text);
            let mut top_rect = RECT { left: 0, top: 0, right: w, bottom: mid };
            windows::Win32::Graphics::Gdi::DrawTextW(
                hdc_mem, &mut wtext, &mut top_rect,
                DT_CENTER | windows::Win32::Graphics::Gdi::DT_BOTTOM | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
            let mut wsub = to_wide(&state.subtext);
            let mut bot_rect = RECT { left: 0, top: mid, right: w, bottom: h };
            windows::Win32::Graphics::Gdi::DrawTextW(
                hdc_mem, &mut wsub, &mut bot_rect,
                DT_CENTER | windows::Win32::Graphics::Gdi::DT_TOP | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
        }

        let _ = SelectObject(hdc_mem, old_font);
        let _ = DeleteObject(font.into());
    }

    let pt_src = POINT { x: 0, y: 0 };
    let mut ppt_dst = POINT { x: rect.left, y: rect.top };
    let mut size = SIZE { cx: w, cy: h };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };

    let _ = UpdateLayeredWindow(
        hwnd,
        Some(hdc_screen),
        Some(&ppt_dst),
        Some(&size),
        Some(hdc_mem),
        Some(&pt_src),
        windows::Win32::Foundation::COLORREF(0),
        Some(&blend),
        UPDATE_LAYERED_WINDOW_FLAGS(2), // ULW_ALPHA
    );

    let _ = SelectObject(hdc_mem, old_bmp);
    let _ = DeleteObject(hbitmap.into());
    let _ = DeleteDC(hdc_mem);
    let _ = ReleaseDC(None, hdc_screen);
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
            let mut ps = PAINTSTRUCT::default();
            unsafe {
                let _ = BeginPaint(hwnd, &mut ps);
                let _ = EndPaint(hwnd, &mut ps);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            unsafe {
                reposition_in_taskbar(hwnd);
                let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize;
                if raw != 0 {
                    let state_ref = &*(raw as *const Mutex<OverlayState>);
                    let guard = state_ref.lock().unwrap();
                    update_layered(hwnd, &guard);
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

            // 查找 Shell_TrayWnd 作为父窗口
            let taskbar = match FindWindowW(w!("Shell_TrayWnd"), None) {
                Ok(t) if !t.is_invalid() => t,
                _ => {
                    log::error!("overlay: 未找到 Shell_TrayWnd");
                    return;
                }
            };
            log::debug!("overlay: 找到 Shell_TrayWnd {:?}", taskbar);

            // 先创建 WS_POPUP 顶层窗口（无父窗口），之后再 SetParent 嵌入任务栏
            let hwnd = match CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE,
                CLASS_NAME,
                windows::core::PCWSTR::null(),
                WS_POPUP,
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

            // 将窗口嵌入 Shell_TrayWnd
            let prev = SetParent(hwnd, Some(taskbar));
            log::debug!("overlay: SetParent 返回 {:?}", prev);

            let leaked = Arc::into_raw(state_thread.clone());
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, leaked as isize);

            // 初始定位 + 渲染
            reposition_in_taskbar(hwnd);
            let state_ref = &*leaked;
            let guard = state_ref.lock().unwrap();
            update_layered(hwnd, &guard);
            drop(guard);
            log::debug!("overlay: 初始定位 + UpdateLayeredWindow 完成");

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
                unsafe {
                    let guard = self.state.lock().unwrap();
                    update_layered(hwnd, &guard);
                }
            }
        }
    }
}

#[allow(dead_code)]
fn _duration_marker(_: Duration) {}
