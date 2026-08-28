use crate::lyrics::lrc::Lyrics;
use crate::lyrics::{lrclib, netease};

/// 为某首歌曲解析歌词。依据源播放器决定各歌词源的尝试顺序。
///
/// * `prefer_netease` —— 当当前会话来自网易云音乐时优先尝试它，
///   以便拿到与 App 本身显示一致的歌词（含翻译）。
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
