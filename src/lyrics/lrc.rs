use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct LyricLine {
    pub time: Duration,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
    /// LRC [offset:] in milliseconds (signed; applied as an addition to line times)
    pub offset_ms: i64,
}

fn parse_time(s: &str) -> Option<Duration> {
    let s = s.trim();
    let parts: Vec<&str> = s.split(':').collect();
    let parse_sec = |p: &str| -> Option<f64> {
        let p = p.trim();
        if let Some((a, b)) = p.split_once('.') {
            Some(format!("{}.{}", a, b).parse().ok()?)
        } else if let Some((a, b)) = p.split_once(',') {
            Some(format!("{}.{}", a, b).parse().ok()?)
        } else {
            p.parse().ok()
        }
    };
    match parts.len() {
        2 => {
            let m: f64 = parts[0].parse().ok()?;
            let sec: f64 = parse_sec(parts[1])?;
            Some(Duration::from_secs_f64(m * 60.0 + sec))
        }
        3 => {
            let h: f64 = parts[0].parse().ok()?;
            let m: f64 = parts[1].parse().ok()?;
            let sec: f64 = parse_sec(parts[2])?;
            Some(Duration::from_secs_f64(h * 3600.0 + m * 60.0 + sec))
        }
        _ => None,
    }
}

fn parse_offset(s: &str) -> Option<i64> {
    let s = s.trim();
    s.strip_prefix("offset:")
        .and_then(|v| v.trim().parse::<i64>().ok())
}

pub fn parse(lrc: &str) -> Lyrics {
    let mut lines = Vec::new();
    let mut offset_ms = 0i64;
    for line in lrc.lines() {
        let mut stamps = Vec::new();
        let mut rest = line;
        loop {
            let open = match rest.find('[') {
                Some(i) => i,
                None => break,
            };
            let after = &rest[open + 1..];
            let close = match after.find(']') {
                Some(i) => i,
                None => break,
            };
            let tag = &after[..close];
            if let Some(t) = parse_time(tag) {
                stamps.push(t);
            } else if let Some(o) = parse_offset(tag) {
                offset_ms = o;
            }
            rest = &after[close + 1..];
        }
        let text = rest.trim();
        if text.is_empty() {
            continue;
        }
        for t in stamps {
            lines.push(LyricLine {
                time: t,
                text: text.to_string(),
            });
        }
    }
    lines.sort_by_key(|l| l.time);
    Lyrics { lines, offset_ms }
}

/// Format a duration back to `[mm:ss.xx]` for re-serializing merged LRC.
pub fn fmt_time(d: Duration) -> String {
    let total_ms = d.as_millis();
    let min = total_ms / 60000;
    let sec = (total_ms % 60000) / 1000;
    let cs = (total_ms % 1000) / 10;
    format!("{:02}:{:02}.{:02}", min, sec, cs)
}

/// Merge a translation LRC under the matching original lines (by timestamp).
pub fn merge_bilingual(orig: &str, trans: &str) -> String {
    if trans.trim().is_empty() {
        return orig.to_string();
    }
    let o = parse(orig);
    let t = parse(trans);
    let tmap: HashMap<u128, &str> = t
        .lines
        .iter()
        .map(|l| (l.time.as_millis(), l.text.as_str()))
        .collect();
    let mut out = String::new();
    for l in &o.lines {
        out.push_str(&format!("[{}]{}", fmt_time(l.time), l.text));
        out.push('\n');
        if let Some(tr) = tmap.get(&l.time.as_millis()) {
            if !tr.is_empty() && tr != &l.text.as_str() {
                out.push_str(&format!("[{}]{}\n", fmt_time(l.time), tr));
            }
        }
    }
    out
}
