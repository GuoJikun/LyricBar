use std::time::Duration;

use crate::lyrics::lrc::Lyrics;

/// 将解析后的 LRC 与播放进度同步，并定位当前应显示的歌词行。
pub struct LyricEngine {
    pub lyrics: Lyrics,
    /// 用户可调的全局偏移量（在查找当前行之前叠加到播放进度上）。
    pub user_offset: Duration,
}

impl LyricEngine {
    pub fn new(lyrics: Lyrics) -> Self {
        Self {
            lyrics,
            user_offset: Duration::ZERO,
        }
    }

    /// 应用 LRC 的 [offset:] 偏移与用户偏移后的有效进度。
    ///
    /// `[offset:+N]` 表示歌词整体延后 N 毫秒，需将播放进度加上 N；
    /// `[offset:-N]` 表示歌词整体提前 N 毫秒，需将播放进度减去 N。
    fn adjusted(&self, position: Duration) -> Duration {
        let pos = position.saturating_add(self.user_offset);
        if self.lyrics.offset_ms >= 0 {
            pos.saturating_add(Duration::from_millis(self.lyrics.offset_ms as u64))
        } else {
            pos.saturating_sub(Duration::from_millis((-self.lyrics.offset_ms) as u64))
        }
    }

    /// `position` 对应的当前歌词行下标；在第一条时间戳之前返回 None。
    ///
    /// 当翻译行与原歌词行拥有相同时间戳（来自 `merge_bilingual`）时，
    /// 我们返回该重复组的第一行，使原歌词文本被视为当前活动行。
    pub fn current_line(&self, position: Duration) -> Option<usize> {
        let adj = self.adjusted(position);
        let mut idx = None;
        for (i, line) in self.lyrics.lines.iter().enumerate() {
            if line.time <= adj {
                idx = Some(i);
            } else {
                break;
            }
        }
        if let Some(mut i) = idx {
            while i > 0 && self.lyrics.lines[i - 1].time == self.lyrics.lines[i].time {
                i -= 1;
            }
            return Some(i);
        }
        None
    }

    /// 当前活动行文本；若无则返回空字符串。
    pub fn current_text(&self, position: Duration) -> String {
        match self.current_line(position) {
            Some(i) => self.lyrics.lines[i].text.clone(),
            None => String::new(),
        }
    }

    /// 当前行与下一行（用于双语显示或下一句预览）；若无可用的下一行则为空。
    pub fn current_pair(&self, position: Duration) -> (String, String) {
        match self.current_line(position) {
            Some(i) => {
                let main = self.lyrics.lines[i].text.clone();
                let next = self
                    .lyrics
                    .lines
                    .get(i + 1)
                    .map(|l| l.text.clone())
                    .unwrap_or_default();
                (main, next)
            }
            None => (String::new(), String::new()),
        }
    }
}
