use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct MediaMetadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub source_app: String,
    pub playback_status: PlaybackStatus,
    pub position: Duration,
    pub duration: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackStatus {
    #[default]
    Closed,
    Playing,
    Paused,
    Stopped,
}

impl PlaybackStatus {
    pub fn is_active(self) -> bool {
        matches!(self, PlaybackStatus::Playing | PlaybackStatus::Paused)
    }
}
