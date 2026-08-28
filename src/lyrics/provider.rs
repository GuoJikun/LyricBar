use crate::lyrics::lrc::Lyrics;
use crate::lyrics::{lrclib, netease};

/// Resolve lyrics for a track. Tries providers in an order based on the source app.
///
/// * `prefer_netease` — when the active session is NetEase Cloud Music, try it first
///   so we get the same lyrics (incl. translation) the app itself shows.
pub async fn resolve(
    client: &reqwest::Client,
    title: &str,
    artist: &str,
    prefer_netease: bool,
) -> anyhow::Result<Option<Lyrics>> {
    let order: &[&str] = if prefer_netease {
        &["netease", "lrclib"]
    } else {
        &["lrclib", "netease"]
    };

    for name in order {
        let text = match *name {
            "netease" => netease::fetch(client, title, artist).await,
            _ => lrclib::fetch(client, title, artist).await,
        };
        match text {
            Ok(Some(t)) => {
                let lyrics = crate::lyrics::lrc::parse(&t);
                if !lyrics.lines.is_empty() {
                    return Ok(Some(lyrics));
                }
            }
            Ok(None) => continue,
            Err(e) => eprintln!("lyrics provider {name} error: {e}"),
        }
    }
    Ok(None)
}
