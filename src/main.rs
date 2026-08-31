#![windows_subsystem = "windows"]

mod crash;
mod log_writer;

use std::time::{Duration, Instant};

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
    let tray = lyricbar::ui::tray::spawn_tray()?;
    log::info!("SMTC / 悬浮窗 / 托盘 初始化完成");

    let mut engine: Option<LyricEngine> = None;
    let mut last_key = String::new();

    // 自行追踪播放进度，弥补部分播放器 SMTC Position() 不更新的问题
    let mut tracked_position = Duration::ZERO;
    let mut track_anchor: Option<Instant> = None; // 上次校准时刻
    let mut last_status_playing = false;

    log::info!("进入主循环，开始监听 SMTC");

    while lyricbar::ui::tray::is_running() {
        match smtc.current().await {
            Ok(Some(m)) => {
                let key = format!("{}\u{1f}\u{1f}{}", m.title, m.artist);
                let prefer_netease = m.source_app.to_lowercase().contains("netease");
                let is_playing = m.playback_status.is_active();

                if key != last_key {
                    last_key = key.clone();
                    let tip = format!("{} - {}", m.title, m.artist);
                    tray.set_tooltip(&tip);
                    log::info!("[song] changed to: {} — {} (position={:?}, duration={:?}, status={:?})",
                        m.title, m.artist, m.position, m.duration, m.playback_status);
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
                    // 歌曲切换 → 用 SMTC 上报的 position 重新校准
                    tracked_position = m.position;
                    track_anchor = if is_playing { Some(Instant::now()) } else { None };
                    last_status_playing = is_playing;
                }

                // 状态变化时校准
                if is_playing && !last_status_playing {
                    // 从暂停/停止恢复播放 → 保持冻结位置，重启计时器
                    track_anchor = Some(Instant::now());
                } else if !is_playing && last_status_playing {
                    // 暂停/停止 → 冻结位置
                    if let Some(anchor) = track_anchor {
                        tracked_position = tracked_position.saturating_add(anchor.elapsed());
                    }
                    track_anchor = None;
                } else if is_playing {
                    // 持续播放 → 用 SMTC position 校准（如果它在前进的话）
                    if m.position > Duration::ZERO {
                        let tracked_elapsed = track_anchor
                            .map(|a| tracked_position.saturating_add(a.elapsed()))
                            .unwrap_or(tracked_position);
                        let diff = if m.position > tracked_elapsed {
                            m.position - tracked_elapsed
                        } else {
                            tracked_elapsed - m.position
                        };
                        if diff < Duration::from_secs(2) || tracked_elapsed < Duration::from_secs(1) {
                            tracked_position = m.position;
                            track_anchor = Some(Instant::now());
                        }
                    }
                }
                last_status_playing = is_playing;

                // 计算最终用于歌词同步的位置
                let effective_position = if is_playing {
                    track_anchor
                        .map(|a| tracked_position.saturating_add(a.elapsed()))
                        .unwrap_or(tracked_position)
                } else {
                    tracked_position
                };

                if is_playing {
                    if let Some(eng) = &engine {
                        let main = eng.current_text(effective_position);
                        log::debug!("[sync] effective={:?} smtc_pos={:?} text={:?}",
                            effective_position, m.position, main);
                        overlay.set_text(&main, "");
                    }
                } else {
                    overlay.set_text("", "");
                }
            }
            Ok(None) => {
                if !last_key.is_empty() {
                    last_key = String::new();
                    engine = None;
                    track_anchor = None;
                    tracked_position = Duration::ZERO;
                    tray.set_tooltip("LyricBar - 歌词悬浮条");
                }
                overlay.set_text("", "");
            }
            Err(e) => log::error!("smtc error: {e}"),
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    log::info!("LyricBar 退出");
    Ok(())
}
