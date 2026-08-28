//! 系统托盘：蓝色图标 + 右键菜单（退出）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleDC, CreateSolidBrush, DeleteDC, DeleteObject, FillRect,
    SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DispatchMessageW, GetMessageW, GetCursorPos, PostMessageW, PostQuitMessage,
    RegisterClassW, SetForegroundWindow, SetWindowLongPtrW, TrackPopupMenu, TranslateMessage,
    GWLP_USERDATA, HICON, ICONINFO, MF_STRING, MSG, TPM_LEFTBUTTON, WNDCLASSW, WM_COMMAND,
    WM_RBUTTONUP, WM_USER, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};

const TRAY_ICON_MSG: u32 = 0x800;
const IDM_EXIT: u32 = 1;
const CLASS_NAME: windows::core::PCWSTR = w!("LyricBarTray");

static RUNNING: AtomicBool = AtomicBool::new(true);

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn create_blue_icon() -> HICON {
    unsafe {
        let hdc = CreateCompatibleDC(None);
        let bmp = CreateBitmap(16, 16, 1, 32, Some(std::ptr::null()));
        let old_bmp = SelectObject(hdc, bmp.into());

        let brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x00DD9933));
        let mut rect = RECT { left: 0, top: 0, right: 16, bottom: 16 };
        FillRect(hdc, &mut rect, brush);
        let _ = DeleteObject(brush.into());

        SelectObject(hdc, old_bmp);
        let _ = DeleteDC(hdc);

        let mut icon_info = ICONINFO {
            fIcon: windows::core::BOOL(1),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: bmp,
            hbmColor: bmp,
        };
        let icon = CreateIconIndirect(&mut icon_info).unwrap();
        let _ = DeleteObject(bmp.into());
        icon
    }
}

fn modify_tip(hwnd: HWND, tip: &str) {
    unsafe {
        let wide_tip = to_wide(tip);
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_TIP,
            ..Default::default()
        };
        let tip_len = wide_tip.len().min(128);
        nid.szTip[..tip_len].copy_from_slice(&wide_tip[..tip_len]);
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        msg if msg == TRAY_ICON_MSG => {
            if lparam.0 as u32 == WM_RBUTTONUP {
                unsafe {
                    let mut pt = windows::Win32::Foundation::POINT::default();
                    let _ = GetCursorPos(&mut pt);
                    let menu = CreatePopupMenu().unwrap();
                    let exit_text = to_wide("退出(&X)");
                    let _ = AppendMenuW(menu, MF_STRING, IDM_EXIT as usize, windows::core::PCWSTR(exit_text.as_ptr()));
                    SetForegroundWindow(hwnd);
                    let _ = TrackPopupMenu(menu, TPM_LEFTBUTTON, pt.x, pt.y, Some(hwnd.0 as i32), hwnd, None);
                    let _ = PostMessageW(Some(hwnd), WM_USER, WPARAM(0), LPARAM(0));
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            if wparam.0 as u32 == IDM_EXIT {
                unsafe {
                    RUNNING.store(false, Ordering::Relaxed);
                    let mut nid = NOTIFYICONDATAW {
                        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                        hWnd: hwnd,
                        uID: 1,
                        ..Default::default()
                    };
                    let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
                    PostQuitMessage(0);
                }
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

enum TrayCommand {
    SetTooltip(String),
}

pub struct TrayHandle {
    tx: mpsc::Sender<TrayCommand>,
}

impl TrayHandle {
    pub fn set_tooltip(&self, tip: &str) {
        let _ = self.tx.send(TrayCommand::SetTooltip(tip.to_string()));
    }
}

pub fn is_running() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

pub fn spawn_tray() -> anyhow::Result<TrayHandle> {
    let (tx, rx) = mpsc::channel();

    std::thread::Builder::new()
        .name("tray".into())
        .spawn(move || {
            unsafe {
                let hinst = GetModuleHandleW(None).unwrap_or_default();
                let wc = WNDCLASSW {
                    lpfnWndProc: Some(wndproc),
                    hInstance: hinst.into(),
                    lpszClassName: CLASS_NAME,
                    ..Default::default()
                };
                RegisterClassW(&wc);

                let hwnd = match CreateWindowExW(
                    WS_EX_TOOLWINDOW,
                    CLASS_NAME,
                    w!("LyricBarTray"),
                    WS_OVERLAPPED,
                    0, 0, 0, 0,
                    None, None, Some(hinst.into()), None,
                ) {
                    Ok(h) => h,
                    Err(e) => {
                        log::error!("托盘窗口创建失败: {e}");
                        return;
                    }
                };
                if hwnd.is_invalid() {
                    log::error!("托盘窗口句柄无效");
                    return;
                }

                SetWindowLongPtrW(hwnd, GWLP_USERDATA, hwnd.0 as isize);

                let icon = create_blue_icon();

                let mut nid = NOTIFYICONDATAW {
                    cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                    hWnd: hwnd,
                    uID: 1,
                    uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
                    uCallbackMessage: TRAY_ICON_MSG,
                    hIcon: icon,
                    ..Default::default()
                };
                let tip = to_wide("LyricBar - 歌词悬浮条");
                let tip_len = tip.len().min(128);
                nid.szTip[..tip_len].copy_from_slice(&tip[..tip_len]);
                let _ = Shell_NotifyIconW(NIM_ADD, &nid);
                log::info!("托盘图标已创建");

                let mut msg = MSG::default();
                while RUNNING.load(Ordering::Relaxed) {
                    // 处理托盘命令
                    while let Ok(cmd) = rx.try_recv() {
                        match cmd {
                            TrayCommand::SetTooltip(tip) => modify_tip(hwnd, &tip),
                        }
                    }

                    // 非阻塞取消息
                    if GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        let _ = TranslateMessage(&msg);
                        let _ = DispatchMessageW(&msg);
                    } else {
                        break;
                    }
                }

                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
                DestroyIcon(icon).ok();
                log::info!("托盘已退出");
            }
        })?;

    Ok(TrayHandle { tx })
}
