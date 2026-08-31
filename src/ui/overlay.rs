//! 歌词悬浮窗口，使用 WebView2 渲染歌词 HTML，
//! 通过 SetParent(Shell_TrayWnd) 嵌入任务栏。
//! WebView2 使用 DirectComposition surface，DWM 能正确合成。

use std::sync::{mpsc, Arc, Mutex};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{self as wv2, CreateCoreWebView2EnvironmentCompletedHandler,
    CreateCoreWebView2ControllerCompletedHandler};

pub const LOGICAL_WIDTH: f64 = 240.0;
pub const LOGICAL_HEIGHT: f64 = 28.0;

/// WebView2 线程上的所有资源（非 Send，只能在创建线程上访问）。
struct ThreadState {
    rx: mpsc::Receiver<String>,
    webview: ICoreWebView2,
    #[allow(dead_code)]
    controller: ICoreWebView2Controller,
}

pub struct Overlay {
    hwnd: Arc<Mutex<Option<usize>>>,
    tx: mpsc::Sender<String>,
}

const CLASS_NAME: PCWSTR = w!("LyricBarOverlay");

fn build_html(text: &str, subtext: &str) -> String {
    if text.is_empty() && subtext.is_empty() {
        return r#"<!DOCTYPE html>
<html><head><style>
* { margin:0; padding:0; }
html, body { width:100%; height:100%; overflow:hidden; background:transparent; }
</style></head><body></body></html>"#.to_string();
    }
    if subtext.is_empty() {
        format!(
            r#"<!DOCTYPE html>
<html><head><style>
* {{ margin:0; padding:0; }}
html, body {{ width:100%; height:100%; overflow:hidden; background:transparent; }}
body {{ display:flex; justify-content:center; align-items:center; }}
.text {{ font-family:"Microsoft YaHei",sans-serif; font-size:14px; font-weight:bold;
         color:white; text-shadow:0 1px 3px rgba(0,0,0,0.8); white-space:nowrap;
         overflow:hidden; text-overflow:ellipsis; max-width:100%; }}
</style></head><body><div class="text">{}</div></body></html>"#,
            html_escape(text)
        )
    } else {
        format!(
            r#"<!DOCTYPE html>
<html><head><style>
* {{ margin:0; padding:0; }}
html, body {{ width:100%; height:100%; overflow:hidden; background:transparent; }}
.container {{ width:100%; height:100%; display:flex; flex-direction:column; justify-content:center; }}
.text {{ font-family:"Microsoft YaHei",sans-serif; font-size:13px; font-weight:bold;
         color:white; text-shadow:0 1px 3px rgba(0,0,0,0.8); white-space:nowrap;
         overflow:hidden; text-overflow:ellipsis; max-width:100%; line-height:1.2; }}
.subtext {{ font-family:"Microsoft YaHei",sans-serif; font-size:11px;
           color:rgba(255,255,255,0.7); text-shadow:0 1px 2px rgba(0,0,0,0.7);
           white-space:nowrap; overflow:hidden; text-overflow:ellipsis;
           max-width:100%; line-height:1.2; }}
</style></head><body><div class="container">
  <div class="text">{}</div>
  <div class="subtext">{}</div>
</div></body></html>"#,
            html_escape(text),
            html_escape(subtext)
        )
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
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

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER => {
                reposition_in_taskbar(hwnd);
                // pump 内容更新
                let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ThreadState;
                if !raw.is_null() {
                    let state = &mut *raw;
                    while let Ok(html) = state.rx.try_recv() {
                        let hstring = HSTRING::from(&html);
                        let _ = state.webview.NavigateToString(&hstring);
                        log::debug!("WebView2: 更新 HTML ({} bytes)", html.len());
                    }
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
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
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

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

            // DwmExtendFrameIntoClientArea — 将 DWM frame 扩展到整个客户区，实现完全透明背景
            {
                let margins = windows::Win32::UI::Controls::MARGINS {
                    cxLeftWidth: -1,
                    cxRightWidth: -1,
                    cyTopHeight: -1,
                    cyBottomHeight: -1,
                };
                let _ = windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea(hwnd, &margins);
            }

            // WebView2 环境
            let environment = {
                let (etx, erx) = mpsc::channel();
                let _ = CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
                    Box::new(|handler| unsafe {
                        CreateCoreWebView2Environment(&handler)
                            .map_err(wv2::Error::WindowsError)
                    }),
                    Box::new(move |error_code, environment| {
                        if let Err(e) = error_code {
                            let _ = etx.send(Err(e));
                            return Ok(());
                        }
                        let _ = etx.send(environment.ok_or_else(|| windows::core::Error::from(E_POINTER)));
                        Ok(())
                    }),
                );
                match erx.recv() {
                    Ok(Ok(env)) => env,
                    Ok(Err(e)) => {
                        log::error!("overlay: WebView2 环境失败: {e}");
                        return;
                    }
                    Err(_) => return,
                }
            };
            log::debug!("overlay: WebView2 环境创建成功");

            // WebView2 控制器
            let controller = {
                let (etx, erx) = mpsc::channel();
                let _ = CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
                    Box::new(move |handler| unsafe {
                        environment
                            .CreateCoreWebView2Controller(hwnd, &handler)
                            .map_err(wv2::Error::WindowsError)
                    }),
                    Box::new(move |error_code, controller| {
                        if let Err(e) = error_code {
                            let _ = etx.send(Err(e));
                            return Ok(());
                        }
                        let _ = etx.send(controller.ok_or_else(|| windows::core::Error::from(E_POINTER)));
                        Ok(())
                    }),
                );
                match erx.recv() {
                    Ok(Ok(ctrl)) => ctrl,
                    Ok(Err(e)) => {
                        log::error!("overlay: WebView2 控制器失败: {e}");
                        return;
                    }
                    Err(_) => return,
                }
            };
            log::debug!("overlay: WebView2 控制器创建成功");

            let _ = controller.SetBounds(RECT { left: 0, top: 0, right: 240, bottom: 28 });
            let _ = controller.SetIsVisible(true);

            let webview = match controller.CoreWebView2() {
                Ok(wv) => wv,
                Err(e) => {
                    log::error!("overlay: CoreWebView2() 失败: {e}");
                    return;
                }
            };

            if let Ok(settings) = webview.Settings() {
                let _ = settings.SetAreDefaultContextMenusEnabled(false);
                let _ = settings.SetAreDevToolsEnabled(false);
            }

            // 初始空白页
            let html = HSTRING::from(build_html("", ""));
            let _ = webview.NavigateToString(&html);
            log::debug!("overlay: WebView2 初始化完成");

            // 嵌入任务栏
            let prev = SetParent(hwnd, Some(taskbar));
            log::debug!("overlay: SetParent 返回 {:?}", prev);
            let _ = ShowWindow(hwnd, SW_SHOWNA);

            // 将 ThreadState 存到栈上，用裸指针传给 GWLP_USERDATA
            let mut thread_state = ThreadState { rx, webview, controller };
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

            // thread_state 在此自动 drop
            drop(thread_state);
            let _ = CoUninitialize();
        });

        Ok(Self { hwnd: hwnd_slot, tx })
    }

    pub fn set_text(&self, text: &str, subtext: &str) {
        let html = build_html(text, subtext);
        log::debug!("set_text: 主={:?} 副={:?}", text, subtext);
        let _ = self.tx.send(html);
    }
}
