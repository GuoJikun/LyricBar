mod crash;
mod log_writer;

use std::time::Duration;

use lyricbar::lyrics::engine::LyricEngine;
use lyricbar::lyrics::provider;
use lyricbar::smtc::Smtc;
use lyricbar::ui::overlay::Overlay;

/// 初始化日志系统（同时输出到控制台和文件，支持日志轮转）
fn setup_log() -> anyhow::Result<()> {
    let exe_dir = std::env::current_exe()?
        .parent()
        .ok_or_else(|| anyhow::anyhow!("无法获取可执行文件目录"))?
        .to_path_buf();

    let logs_dir = exe_dir.join("logs");
    std::fs::create_dir_all(&logs_dir)?;

    let log_writer = log_writer::RotatingWriter::new(logs_dir, "LyricBar.log", 1024 * 1024, 5);

    let mut dispatch = fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{}][{}][{}] {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                record.level(),
                record.target(),
                message
            ))
        })
        .level(log::LevelFilter::Debug)
        .chain(Box::new(log_writer) as Box<dyn std::io::Write + Send>);

    // 开发模式下同时输出到控制台，正式运行仅写文件日志
    if cfg!(debug_assertions) {
        dispatch = dispatch.chain(std::io::stdout());
    }

    dispatch.apply()?;

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    setup_log()?;

    // 日志初始化完成后立即安装崩溃捕获，确保 panic/原生异常能写入日志
    crash::install();

    log::info!("========================================");
    log::info!("LyricBar 已启动");
    log::info!("========================================");

    let client = reqwest::Client::builder().build()?;
    let smtc = Smtc::new().await?;
    let overlay = Overlay::new()?;
    lyricbar::ui::tray::init()?;
    log::info!("SMTC / 悬浮窗 / 托盘 初始化完成");

    let mut engine: Option<LyricEngine> = None;
    let mut last_key = String::new();

    log::info!("进入主循环，开始监听 SMTC");

    loop {
        match smtc.current().await {
            Ok(Some(m)) => {
                let key = format!("{}\u{1f}\u{1f}{}", m.title, m.artist);
                let prefer_netease = m.source_app.to_lowercase().contains("netease");

                if key != last_key {
                    last_key = key.clone();
                    match provider::resolve(&client, &m.title, &m.artist, prefer_netease).await {
                        Ok(Some(lyrics)) => {
                            engine = Some(LyricEngine::new(lyrics));
                            log::info!("[lyrics] loaded for: {} — {}", m.title, m.artist);
                        }
                        Ok(None) => {
                            engine = None;
                            overlay.set_text("暂无歌词", "");
                            log::info!("[lyrics] none found for: {} — {}", m.title, m.artist);
                        }
                        Err(e) => log::error!("lyrics resolve error: {e}"),
                    }
                }

                if m.playback_status.is_active() {
                    if let Some(eng) = &engine {
                        let (main, sub) = eng.current_pair(m.position);
                        overlay.set_text(&main, &sub);
                    }
                } else {
                    overlay.set_text("", "");
                }
            }
            Ok(None) => {
                if !last_key.is_empty() {
                    last_key = String::new();
                    engine = None;
                }
                overlay.set_text("", "");
            }
            Err(e) => log::error!("smtc error: {e}"),
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
