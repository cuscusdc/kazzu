//! Playback vocabulary shared by the audio engine, the UI and (later) remote
//! control / listen-together: queue items, commands, events and state.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::ids::QueueItemId;
use crate::model::{AudioFormat, Track, TrackSource};
use crate::provider::StreamSource;

/// One entry of the play queue: track metadata plus an openable stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueueItem {
    pub id: QueueItemId,
    pub track: Track,
    pub stream: StreamSource,
}

impl QueueItem {
    /// New queue entry with a fresh [`QueueItemId`].
    pub fn new(track: Track, stream: StreamSource) -> Self {
        Self {
            id: QueueItemId::next(),
            track,
            stream,
        }
    }

    /// Queue entry for a local track (stream = the track's own file).
    /// Returns `None` for remote tracks, which must be resolved first.
    pub fn local(track: Track) -> Option<Self> {
        match &track.source {
            TrackSource::Local { path } => {
                let stream = StreamSource::file(path.clone());
                Some(Self::new(track, stream))
            }
            TrackSource::Remote { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackStatus {
    #[default]
    Stopped,
    /// Waiting for data (opening a file, network buffering).
    Buffering,
    Playing,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatMode {
    #[default]
    Off,
    /// Repeat the current track.
    One,
    /// Repeat the whole queue.
    All,
}

/// What actually reaches the output device. Lets the UI show "bit-perfect"
/// or "resampled 96 kHz → 48 kHz".
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OutputInfo {
    pub device: String,
    pub sample_rate: u32,
    pub channels: u16,
    /// Source format of the current track.
    pub source: Option<AudioFormat>,
    /// True when no resampling and no DSP alters the samples.
    pub bit_perfect: bool,
    /// Some(source rate) when the engine had to resample.
    pub resampled_from: Option<u32>,
}

/// Commands accepted by the playback engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "cmd")]
pub enum PlaybackCommand {
    /// Replace the queue and start playing `start` (index into `items`).
    Load { items: Vec<QueueItem>, start: usize },
    /// Append to the end of the queue.
    Enqueue { items: Vec<QueueItem> },
    /// Insert right after the current item.
    PlayNext { items: Vec<QueueItem> },
    Play,
    Pause,
    TogglePause,
    Stop,
    Seek { position: Duration },
    /// Linear gain 0.0..=1.0 (1.0 = unity = bit-perfect).
    SetVolume { volume: f32 },
    Next,
    /// Restarts the current track if past a few seconds, else goes back.
    Previous,
    JumpTo { id: QueueItemId },
    Remove { id: QueueItemId },
    /// Move an item immediately before another item in playback order.
    MoveBefore { id: QueueItemId, before: QueueItemId },
    Clear,
    SetRepeat { mode: RepeatMode },
    SetShuffle { enabled: bool },
}

/// Events emitted by the playback engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "event")]
pub enum PlaybackEvent {
    StatusChanged {
        status: PlaybackStatus,
    },
    /// The current item changed (`None` = nothing loaded).
    TrackChanged {
        item: Option<Box<QueueItem>>,
        duration: Option<Duration>,
    },
    /// Periodic position update (about 10 Hz while playing).
    Position {
        position: Duration,
        duration: Option<Duration>,
    },
    /// An item played to its end (not emitted on skip/stop).
    TrackEnded {
        id: QueueItemId,
    },
    /// Queue contents or order changed.
    QueueChanged {
        items: Vec<QueueItem>,
        current: Option<usize>,
    },
    VolumeChanged {
        volume: f32,
    },
    RepeatChanged {
        mode: RepeatMode,
    },
    ShuffleChanged {
        enabled: bool,
    },
    /// The output device/format was (re)configured.
    OutputChanged {
        info: OutputInfo,
    },
    /// Reached the end of the queue.
    QueueFinished,
    /// The sleep timer has paused playback.
    SleepTimerElapsed,
    /// Non-fatal error (e.g. a track failed to decode and was skipped).
    Error {
        id: Option<QueueItemId>,
        message: String,
    },
}

/// Full snapshot of the player, e.g. for a newly attached UI or remote.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PlaybackState {
    pub status: PlaybackStatus,
    pub queue: Vec<QueueItem>,
    pub current: Option<usize>,
    pub position: Duration,
    pub duration: Option<Duration>,
    pub volume: f32,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    pub output: Option<OutputInfo>,
}

impl PlaybackState {
    pub fn current_item(&self) -> Option<&QueueItem> {
        self.current.and_then(|i| self.queue.get(i))
    }
}
