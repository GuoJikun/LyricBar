use reqwest::Client;

pub async fn fetch(client: &Client, title: &str, artist: &str) -> anyhow::Result<Option<String>> {
    let resp = client
        .get("https://lrclib.net/api/search")
        .query(&[("artist", artist), ("track", title)])
        .send()
        .await?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let value: serde_json::Value = resp.json().await?;
    let arr = match value {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Object(o) => vec![serde_json::Value::Object(o)],
        _ => return Ok(None),
    };
    for item in &arr {
        if let Some(synced) = item.get("syncedLyrics").and_then(|v| v.as_str()) {
            if !synced.trim().is_empty() {
                return Ok(Some(synced.to_string()));
            }
        }
    }
    for item in &arr {
        if let Some(plain) = item.get("plainLyrics").and_then(|v| v.as_str()) {
            if !plain.trim().is_empty() {
                return Ok(Some(plain.to_string()));
            }
        }
    }
    Ok(None)
}
