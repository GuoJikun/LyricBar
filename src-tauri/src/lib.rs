mod crash;
mod lyrics;
mod overlay;
mod smtc;

use std::time::{Duration, Instant};

use lyrics::engine::LyricEngine;
use lyrics::provider;
use smtc::Smtc;
use tauri::Emitter;
use tauri_plugin_log::{Target, TargetKind};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir { file_name: None }),
                    Target::new(TargetKind::Webview),
                ])
                .max_file_size(1_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(5))
                .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
                .level(log::LevelFilter::Debug)
                .build(),
        )
        .setup(|app| {
            log::info!("========================================");
            log::info!("LyricBar 已启动");
            log::info!("========================================");

            crash::install();

            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = run_lyric_sync(app_handle).await {
                    log::error!("歌词同步错误: {e}");
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

async fn run_lyric_sync(app: tauri::AppHandle) -> anyhow::Result<()> {
    let client = reqwest::Client::builder().build()?;
    let smtc = Smtc::new().await?;

    let mut engine: Option<LyricEngine> = None;
    let mut last_key = String::new();
    let mut tracked_position = Duration::ZERO;
    let mut track_anchor: Option<Instant> = None;
    let mut last_status_playing = false;

    log::info!("进入主循环，开始监听 SMTC");

    loop {
        match smtc.current().await {
            Ok(Some(m)) => {
                let key = format!("{}\u{1f}\u{1f}{}", m.title, m.artist);
                let prefer_netease = m.source_app.to_lowercase().contains("netease");
                let is_playing = m.playback_status.is_active();

                if key != last_key {
                    last_key = key.clone();
                    log::info!("[song] changed to: {} — {} (position={:?}, duration={:?}, status={:?})",
                        m.title, m.artist, m.position, m.duration, m.playback_status);

                    match provider::resolve(&client, &m.title, &m.artist, prefer_netease).await {
                        Ok(Some(lyrics)) => {
                            engine = Some(LyricEngine::new(lyrics));
                            log::info!("[lyrics] loaded for: {} — {}", m.title, m.artist);
                        }
                        Ok(None) => {
                            engine = None;
                            let _ = app.emit("lyric-update", "");
                            log::info!("[lyrics] none found for: {} — {}", m.title, m.artist);
                        }
                        Err(e) => log::error!("lyrics resolve error: {e}"),
                    }

                    tracked_position = m.position;
                    track_anchor = if is_playing { Some(Instant::now()) } else { None };
                    last_status_playing = is_playing;
                }

                if is_playing && !last_status_playing {
                    track_anchor = Some(Instant::now());
                } else if !is_playing && last_status_playing {
                    if let Some(anchor) = track_anchor {
                        tracked_position = tracked_position.saturating_add(anchor.elapsed());
                    }
                    track_anchor = None;
                } else if is_playing {
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
                        let _ = app.emit("lyric-update", &main);
                    }
                } else {
                    let _ = app.emit("lyric-update", "");
                }
            }
            Ok(None) => {
                if !last_key.is_empty() {
                    last_key = String::new();
                    engine = None;
                    track_anchor = None;
                    tracked_position = Duration::ZERO;
                }
                let _ = app.emit("lyric-update", "");
            }
            Err(e) => log::error!("smtc error: {e}"),
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
