use std::time::Duration;

use crate::lyrics::lrc::Lyrics;

pub struct LyricEngine {
    pub lyrics: Lyrics,
    pub user_offset: Duration,
}

impl LyricEngine {
    pub fn new(lyrics: Lyrics) -> Self {
        Self {
            lyrics,
            user_offset: Duration::ZERO,
        }
    }

    fn adjusted(&self, position: Duration) -> Duration {
        let pos = position.saturating_add(self.user_offset);
        if self.lyrics.offset_ms >= 0 {
            pos.saturating_add(Duration::from_millis(self.lyrics.offset_ms as u64))
        } else {
            pos.saturating_sub(Duration::from_millis((-self.lyrics.offset_ms) as u64))
        }
    }

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

    pub fn current_text(&self, position: Duration) -> String {
        match self.current_line(position) {
            Some(i) => self.lyrics.lines[i].text.clone(),
            None => String::new(),
        }
    }

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
