use windows::Win32::System::Diagnostics::Debug::{
    EXCEPTION_POINTERS, SetUnhandledExceptionFilter,
};

pub fn install() {
    install_panic_hook();
    install_unhandled_exception_filter();
    log::info!("崩溃捕获已启用");
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
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

fn install_unhandled_exception_filter() {
    unsafe {
        SetUnhandledExceptionFilter(Some(exception_filter));
    }
}

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

        0
    }
}
