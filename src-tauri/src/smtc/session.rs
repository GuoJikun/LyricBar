use std::time::Duration;

use windows::Foundation::TimeSpan;
use windows::Media::Control::*;

use crate::smtc::metadata::{MediaMetadata, PlaybackStatus};

fn ts_to_duration(ts: TimeSpan) -> Duration {
    Duration::from_nanos(ts.Duration.unsigned_abs() * 100)
}

fn map_status(s: GlobalSystemMediaTransportControlsSessionPlaybackStatus) -> PlaybackStatus {
    match s {
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing => PlaybackStatus::Playing,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Paused => PlaybackStatus::Paused,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Stopped => PlaybackStatus::Stopped,
        _ => PlaybackStatus::Closed,
    }
}

pub struct Smtc {
    manager: GlobalSystemMediaTransportControlsSessionManager,
}

impl Smtc {
    pub async fn new() -> anyhow::Result<Self> {
        let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.get()?;
        Ok(Self { manager })
    }

    pub async fn current(&self) -> anyhow::Result<Option<MediaMetadata>> {
        let session = self.manager.GetCurrentSession()?;

        let props = session.TryGetMediaPropertiesAsync()?.get()?;
        let title = props.Title()?.to_string();
        let artist = props.Artist()?.to_string();
        let album = props.AlbumTitle()?.to_string();
        let album_artist = props.AlbumArtist()?.to_string();

        let source_app = session.SourceAppUserModelId()?.to_string();

        let (position, duration) = match session.GetTimelineProperties() {
            Ok(tl) => {
                let position = tl.Position()?;
                let start = tl.StartTime().unwrap_or_default();
                let end = tl.EndTime().unwrap_or_default();
                let dur = if end.Duration > start.Duration {
                    ts_to_duration(TimeSpan {
                        Duration: end.Duration - start.Duration,
                    })
                } else {
                    Duration::ZERO
                };
                (ts_to_duration(position), dur)
            }
            Err(_) => (Duration::ZERO, Duration::ZERO),
        };

        let playback_status = match session.GetPlaybackInfo() {
            Ok(pi) => map_status(pi.PlaybackStatus()?),
            Err(_) => PlaybackStatus::Closed,
        };

        if playback_status == PlaybackStatus::Closed && title.is_empty() {
            return Ok(None);
        }

        Ok(Some(MediaMetadata {
            title,
            artist,
            album,
            album_artist,
            source_app,
            playback_status,
            position,
            duration,
        }))
    }
}
