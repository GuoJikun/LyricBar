use reqwest::Client;

use crate::lyrics::lrc;

async fn search_id(client: &Client, title: &str, artist: &str) -> anyhow::Result<Option<u64>> {
    let q = format!("{title} {artist}");
    log::debug!("[netease] searching id for: {q}");
    let resp = client
        .get("https://music.163.com/api/search/get")
        .query(&[("type", "1"), ("s", &q)])
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36")
        .header("Referer", "https://music.163.com/")
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        log::warn!("[netease] search failed: status={status}, body={}", &body[..body.len().min(500)]);
        return Ok(None);
    }
    let v: serde_json::Value = resp.json().await?;
    log::debug!("[netease] search response: {}", &v.to_string()[..v.to_string().len().min(1000)]);
    let id = v
        .get("result")
        .and_then(|r| r.get("songs"))
        .and_then(|s| s.as_array())
        .and_then(|a| a.first())
        .and_then(|s| s.get("id"))
        .and_then(|i| i.as_u64());
    match id {
        Some(id) => log::debug!("[netease] found song id: {id}"),
        None => log::warn!("[netease] no song id found in response"),
    }
    Ok(id)
}

pub async fn fetch(client: &Client, title: &str, artist: &str) -> anyhow::Result<Option<String>> {
    let id = match search_id(client, title, artist).await? {
        Some(i) => i,
        None => return Ok(None),
    };
    log::debug!("[netease] fetching lyrics for id={id}");
    let resp = client
        .get("https://music.163.com/api/song/lyric")
        .query(&[("id", id.to_string()), ("lv", "1".to_string()), ("tv", "1".to_string())])
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36")
        .header("Referer", "https://music.163.com/")
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        log::warn!("[netease] lyric fetch failed: status={status}, body={}", &body[..body.len().min(500)]);
        return Ok(None);
    }
    let v: serde_json::Value = resp.json().await?;
    log::debug!("[netease] lyric response keys: {:?}", v.as_object().map(|o| o.keys().collect::<Vec<_>>()));
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
        log::warn!("[netease] lrc is empty, full response: {}", &v.to_string()[..v.to_string().len().min(500)]);
        return Ok(None);
    }
    log::debug!("[netease] got lrc length={}, tlyric length={}", lrc.len(), tlyric.len());
    Ok(Some(lrc::merge_bilingual(lrc, tlyric)))
}
