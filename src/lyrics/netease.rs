use reqwest::Client;

use crate::lyrics::lrc;

async fn search_id(client: &Client, title: &str, artist: &str) -> anyhow::Result<Option<u64>> {
    let q = format!("{title} {artist}");
    let resp = client
        .get("https://music.163.com/api/search/get")
        .query(&[("type", "1"), ("s", &q)])
        .header("User-Agent", "Mozilla/5.0")
        .header("Referer", "https://music.163.com/")
        .send()
        .await?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let v: serde_json::Value = resp.json().await?;
    let id = v
        .get("result")
        .and_then(|r| r.get("songs"))
        .and_then(|s| s.as_array())
        .and_then(|a| a.first())
        .and_then(|s| s.get("id"))
        .and_then(|i| i.as_u64());
    Ok(id)
}

/// 获取网易云音乐的歌词（原歌词 + 翻译，合并为双语 LRC）。
pub async fn fetch(client: &Client, title: &str, artist: &str) -> anyhow::Result<Option<String>> {
    let id = match search_id(client, title, artist).await? {
        Some(i) => i,
        None => return Ok(None),
    };
    let resp = client
        .get("https://music.163.com/api/song/lyric")
        .query(&[("id", id.to_string()), ("tv", "1".to_string())])
        .header("User-Agent", "Mozilla/5.0")
        .header("Referer", "https://music.163.com/")
        .send()
        .await?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let v: serde_json::Value = resp.json().await?;
    let lrc = v
        .get("lrc")
        .and_then(|l| l.get("lyric"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let tlyric = v
        .get("tlyric")
        .and_then(|l| l.get("lyric"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if lrc.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(lrc::merge_bilingual(lrc, tlyric)))
}
