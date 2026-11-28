//! The `Provider` abstraction implemented by every music source.
//!
//! Only the interface lives here; implementations (local, Spotify, YT Music,
//! Tidal, Qobuz) belong to their own crates (stage 2). The trait uses
//! `async-trait` so providers can be stored as `Arc<dyn Provider>` in a
//! registry, which native `async fn` in traits does not allow yet.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::ids::{AlbumId, PlaylistId, TrackId};
use crate::model::{Album, Artist, AudioFormat, Playlist, ProviderKind, Track};

/// What a provider can do; lets the UI hide unsupported actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub search: bool,
    pub stream: bool,
    /// Can download for offline playback.
    pub download: bool,
    pub library: bool,
    pub edit_playlists: bool,
    pub like: bool,
    pub recommendations: bool,
}

/// What to search for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    Track,
    Album,
    Artist,
    Playlist,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuery {
    pub text: String,
    /// Empty means "everything".
    pub kinds: Vec<SearchKind>,
    /// Max results per kind.
    pub limit: u32,
    pub offset: u32,
}

impl SearchQuery {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kinds: Vec::new(),
            limit: 20,
            offset: 0,
        }
    }

    pub fn wants(&self, kind: SearchKind) -> bool {
        self.kinds.is_empty() || self.kinds.contains(&kind)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchResults {
    pub tracks: Vec<Track>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
    pub playlists: Vec<Playlist>,
}

/// User library as seen by one provider.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LibrarySnapshot {
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
    pub playlists: Vec<Playlist>,
    pub liked: Vec<TrackId>,
}

/// A concrete, openable audio stream, produced by [`Provider::resolve_stream`]
/// and consumed by the audio engine.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum StreamSource {
    /// A remote queue entry whose transport is resolved when the engine opens it.
    Deferred,

    /// Local (or already downloaded/cached) file.
    File { path: PathBuf },
    /// HTTP(S) stream. Headers carry provider-specific auth if needed.
    Url {
        url: String,
        headers: Vec<(String, String)>,
        /// Known format, if the provider told us (helps probing).
        format: Option<AudioFormat>,
    },
}

impl StreamSource {
    pub fn file(path: impl Into<PathBuf>) -> Self {
        StreamSource::File { path: path.into() }
    }
}

/// Requested stream quality. Default is the best the provider offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamQuality {
    /// Original quality / highest available (hi-res lossless when possible).
    #[default]
    Original,
    /// Lossless, capped at CD quality (16/44.1).
    Lossless,
    /// Best lossy stream (to save bandwidth).
    High,
    /// Low bandwidth.
    Low,
}

/// A music source. Implementations must be cheap to share (`Arc<dyn Provider>`)
/// and must not perform any network call that the user did not trigger.
#[async_trait]
pub trait Provider: Send + Sync {
    fn kind(&self) -> ProviderKind;

    fn capabilities(&self) -> ProviderCapabilities;

    async fn search(&self, query: &SearchQuery) -> Result<SearchResults>;

    /// Turns a track of this provider into something the engine can open.
    async fn resolve_stream(&self, track: &Track, quality: StreamQuality) -> Result<StreamSource>;

    /// The user's saved albums, artists, playlists and likes.
    async fn library(&self) -> Result<LibrarySnapshot>;

    async fn track(&self, id: &TrackId) -> Result<Track>;

    async fn album_tracks(&self, id: &AlbumId) -> Result<Vec<Track>>;

    async fn playlist_tracks(&self, id: &PlaylistId) -> Result<Vec<Track>>;
}
