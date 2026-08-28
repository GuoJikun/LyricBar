use std::time::Duration;

use crate::lyrics::lrc::Lyrics;

/// Synchronizes a parsed LRC to the playback position and locates the active line.
pub struct LyricEngine {
    pub lyrics: Lyrics,
    /// User-adjustable global offset (added to the position before lookup).
    pub user_offset: Duration,
}

impl LyricEngine {
    pub fn new(lyrics: Lyrics) -> Self {
        Self {
            lyrics,
            user_offset: Duration::ZERO,
        }
    }

    /// Effective position after applying LRC [offset:] and the user offset.
    fn adjusted(&self, position: Duration) -> Duration {
        position
            .saturating_add(self.user_offset)
            .saturating_add(Duration::from_millis(self.lyrics.offset_ms.unsigned_abs()))
    }

    /// Index of the active line for `position`, or None before the first timestamp.
    ///
    /// When a translation line shares the same timestamp as the original (from
    /// `merge_bilingual`), we return the *first* line of that duplicate group so the
    /// original text is treated as the active line.
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

    /// Active line text, or empty string if none.
    pub fn current_text(&self, position: Duration) -> String {
        match self.current_line(position) {
            Some(i) => self.lyrics.lines[i].text.clone(),
            None => String::new(),
        }
    }

    /// Active line plus the following line (for bilingual/next-line preview), if any.
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
