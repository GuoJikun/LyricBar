use std::time::Duration;

use lyricbar::lyrics::engine::LyricEngine;
use lyricbar::lyrics::provider;
use lyricbar::smtc::Smtc;
use lyricbar::ui::overlay::Overlay;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = reqwest::Client::builder().build()?;
    let smtc = Smtc::new().await?;
    let overlay = Overlay::new()?;
    lyricbar::ui::tray::init()?;

    let mut engine: Option<LyricEngine> = None;
    let mut last_key = String::new();

    println!("LyricBar running — monitoring SMTC. Press Ctrl+C to exit.");

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
                            println!("[lyrics] loaded for: {} — {}", m.title, m.artist);
                        }
                        Ok(None) => {
                            engine = None;
                            overlay.set_text("暂无歌词", "");
                            println!("[lyrics] none found for: {} — {}", m.title, m.artist);
                        }
                        Err(e) => eprintln!("lyrics resolve error: {e}"),
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
            Err(e) => eprintln!("smtc error: {e}"),
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
