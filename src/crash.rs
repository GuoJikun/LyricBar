//! 崩溃捕获：记录 panic 与原生异常到日志，便于排查无日志退出问题。
//!
//! 原生异常（如 `STATUS_FATAL_USER_CALLBACK_EXCEPTION` / 访问冲突）常发生在
//! Win32 回调（窗口过程）内部，默认不会留下任何痕迹。这里同时安装 panic hook
//! 和未处理异常过滤器，把崩溃原因写入日志文件。

use windows::Win32::System::Diagnostics::Debug::{
    EXCEPTION_POINTERS, SetUnhandledExceptionFilter,
};

/// 安装崩溃捕获钩子。必须在日志初始化之后调用。
pub fn install() {
    install_panic_hook();
    install_unhandled_exception_filter();
    log::info!("崩溃捕获已启用");
}

/// 安装 panic hook，把 panic 信息写入日志。
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // 先调用默认 hook，确保标准错误也能看到信息
        default_hook(info);

        let location = info
            .location()
            .map(|loc| format!("{}:{}", loc.file(), loc.line()))
            .unwrap_or_else(|| "未知位置".to_string());

        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "未知错误".to_string());

        log::error!("发生 panic: {} (位置: {})", payload, location);
        log::error!("线程信息: {:?}", std::thread::current().name());
    }));
}

/// 安装未处理异常过滤器，把原生异常（访问违规等）写入日志。
fn install_unhandled_exception_filter() {
    unsafe {
        SetUnhandledExceptionFilter(Some(exception_filter));
    }
}

/// 未处理异常回调：记录异常代码与地址后返回 EXCEPTION_CONTINUE_SEARCH，
/// 让系统继续默认处理（弹出错误对话框或终止进程）。
unsafe extern "system" fn exception_filter(exception_info: *const EXCEPTION_POINTERS) -> i32 {
    unsafe {
        if !exception_info.is_null() {
            let record = (*exception_info).ExceptionRecord;
            if !record.is_null() {
                let code = (*record).ExceptionCode.0;
                let address = (*record).ExceptionAddress;
                log::error!("未处理异常: 代码=0x{:08X}, 地址={:?}", code, address);
                log::error!("异常线程: {:?}", std::thread::current().name());
            }
        } else {
            log::error!("未处理异常: 异常信息为空");
        }

        // EXCEPTION_CONTINUE_SEARCH = 0，交由系统默认处理
        0
    }
}
