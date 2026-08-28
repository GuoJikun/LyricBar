//! 透明、可点击穿透的悬浮窗口，使用 GDI 渲染，并借助 sysmon 的
//! `SetParent(Shell_TrayWnd)` 方案锚定到任务栏中。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_QUALITY, DeleteObject,
    DT_BOTTOM, DT_CENTER, DT_END_ELLIPSIS, DT_SINGLELINE, DT_TOP, DT_VCENTER, EndPaint, FillRect,
    InvalidateRect, SelectObject, SetBkMode, SetTextColor, CLIP_DEFAULT_PRECIS, DEFAULT_PITCH,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW,
    GetWindowLongPtrW, RegisterClassW, SetLayeredWindowAttributes, SetTimer, SetWindowLongPtrW,
    ShowWindow, TranslateMessage, GWLP_USERDATA, LWA_COLORKEY, MSG, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_EX_TOPMOST, WS_POPUP, WM_DESTROY,
    WM_PAINT, WM_TIMER, SW_SHOW,
};

use crate::ui::taskbar::{embed_into_taskbar, reposition_in_taskbar};

const KEY_COLOR: windows::Win32::Foundation::COLORREF = windows::Win32::Foundation::COLORREF(0x00FF00FF); // 品红色作为透明色键
const CLASS_NAME: windows::core::PCWSTR = w!("LyricBarOverlay");

/// 可变的共享悬浮窗内容（由主线程更新）。
pub struct OverlayState {
    pub text: String,
    pub subtext: String,
}

impl Default for OverlayState {
    fn default() -> Self {
        Self {
            text: String::new(),
            subtext: String::new(),
        }
    }
}

pub struct Overlay {
    hwnd: Arc<Mutex<Option<usize>>>,
    state: Arc<Mutex<OverlayState>>,
}

/// 为 GDI API 构造以 null 结尾的 UTF-16 字符串。
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 由 RGB 分量构造 COLORREF（0x00BBGGRR）。
fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_TIMER => {
            reposition_in_taskbar(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn paint(hwnd: HWND) {
    unsafe {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Arc<Mutex<OverlayState>>;
    let state = if ptr.is_null() {
        None
    } else {
        Some(&*ptr)
    };

    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rect = RECT::default();
    let _ = GetClientRect(hwnd, &mut rect);

    let brush = CreateSolidBrush(KEY_COLOR);
    let _ = FillRect(hdc, &rect, brush);
    let _ = DeleteObject(brush.into());

    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(hdc, windows::Win32::Foundation::COLORREF(rgb(255, 255, 255)));

    let face = to_wide("Microsoft YaHei");
    let font = CreateFontW(
        16,
        0,
        0,
        0,
        700,
        0,
        0,
        0,
        DEFAULT_CHARSET,
        OUT_DEFAULT_PRECIS,
        CLIP_DEFAULT_PRECIS,
        DEFAULT_QUALITY,
        DEFAULT_PITCH.0 as u32,
        windows::core::PCWSTR(face.as_ptr()),
    );
    let old = SelectObject(hdc, font.into());

    if let Some(state) = state {
        let guard = state.lock().unwrap();
        let text = &guard.text;
        let sub = &guard.subtext;

        if sub.is_empty() {
            let mut wide = to_wide(text);
            let _ = windows::Win32::Graphics::Gdi::DrawTextW(
                hdc,
                &mut wide,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
        } else {
            let mid = rect.top + (rect.bottom - rect.top) / 2;
            let mut top = RECT {
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: mid,
            };
            let mut bottom = RECT {
                left: rect.left,
                top: mid,
                right: rect.right,
                bottom: rect.bottom,
            };
            let mut wtext = to_wide(text);
            let _ = windows::Win32::Graphics::Gdi::DrawTextW(
                hdc,
                &mut wtext,
                &mut top,
                DT_CENTER | DT_BOTTOM | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
            let mut wsub = to_wide(sub);
            let _ = windows::Win32::Graphics::Gdi::DrawTextW(
                hdc,
                &mut wsub,
                &mut bottom,
                DT_CENTER | DT_TOP | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
        }
    }

    let _ = SelectObject(hdc, old);
    let _ = DeleteObject(font.into());
    let _ = EndPaint(hwnd, &mut ps);
    }
}

impl Overlay {
    /// 创建悬浮窗并启动其消息循环线程。
    pub fn new() -> anyhow::Result<Self> {
        let state = Arc::new(Mutex::new(OverlayState::default()));
        let hwnd_slot: Arc<Mutex<Option<usize>>> = Arc::new(Mutex::new(None));

        // 仅注册一次窗口类。
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
            let hwnd = match CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                CLASS_NAME,
                windows::core::PCWSTR::null(),
                WS_POPUP,
                0,
                0,
                240,
                28,
                None,
                None,
                Some(hinst.into()),
                None,
            ) {
                Ok(h) => h,
                Err(_) => return,
            };
            if hwnd.is_invalid() {
                return;
            }

            // 将 Arc 克隆泄漏到 GWLP_USERDATA，供 wndproc 读取。
            let leaked = Arc::into_raw(state_thread.clone());
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, leaked as isize);

            let _ = SetLayeredWindowAttributes(hwnd, KEY_COLOR, 0, LWA_COLORKEY);
            let _ = ShowWindow(hwnd, SW_SHOW);

            embed_into_taskbar(hwnd);
            reposition_in_taskbar(hwnd);
            SetTimer(Some(hwnd), 1, 500, None);

            *hwnd_slot_thread.lock().unwrap() = Some(hwnd.0 as usize);

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                let _ = DispatchMessageW(&msg);
            }

            // 线程退出前回收泄漏的 Arc，使其正常释放。
            let _ = Arc::from_raw(leaked);
        });

        Ok(Self { hwnd: hwnd_slot, state })
    }

    /// 更新显示的文本（主歌词行 + 可选翻译行）。
    pub fn set_text(&self, text: &str, subtext: &str) {
        {
            let mut s = self.state.lock().unwrap();
            s.text = text.to_string();
            s.subtext = subtext.to_string();
        }
        if let Some(v) = *self.hwnd.lock().unwrap() {
            let hwnd = HWND(v as *mut std::ffi::c_void);
            if !hwnd.is_invalid() {
                unsafe {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
        }
    }
}

#[allow(dead_code)]
fn _duration_marker(_: Duration) {}
