//! Portable backup of app preferences and playlists. Audio files and account
//! tokens never become part of this file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use kazzu_library::{Library, PlaylistEntry, Track, TrackId};
use serde::{Deserialize, Serialize};

use crate::config::Config;

const VERSION: u32 = 2;
pub(crate) const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Backup {
    version: u32,
    preferences: Config,
    playlists: Vec<SavedPlaylist>,
    liked: Vec<SavedLiked>,
    history: Vec<SavedHistory>,
}

#[derive(Serialize, Deserialize)]
struct SavedPlaylist {
    name: String,
    entries: Vec<SavedTrack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cover: Option<Vec<u8>>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
enum SavedTrack {
    Local {
        relative: Option<PathBuf>,
        metadata: LocalMetadata,
    },
    Remote {
        track: Box<kazzu_core::Track>,
    },
}

#[derive(Serialize, Deserialize)]
struct SavedLiked {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    title: String,
    artist: String,
    album: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct SavedHistory {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    title: String,
    artist: String,
    album: Option<String>,
    played_at: i64,
    listened_ms: u64,
}

#[derive(Serialize, Deserialize, Eq, Hash, PartialEq)]
struct LocalMetadata {
    title: String,
    artists: Vec<String>,
    album: Option<String>,
    duration_ms: u64,
}

impl From<&Track> for LocalMetadata {
    fn from(track: &Track) -> Self {
        Self {
            title: track.title.clone(),
            artists: track.artists.clone(),
            album: track.album.clone(),
            duration_ms: track.duration_ms,
        }
    }
}

pub(crate) fn export(library: &Library, root: &Path, config: Config) -> Result<String, String> {
    let mut playlists = Vec::new();
    for playlist in library.playlists().map_err(|error| error.to_string())? {
        let entries = library
            .playlist_entries(playlist.id)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|entry| match entry {
                PlaylistEntry::Local(track) => SavedTrack::Local {
                    relative: track.path.strip_prefix(root).ok().map(Path::to_owned),
                    metadata: LocalMetadata::from(&track),
                },
                PlaylistEntry::Remote(track) => SavedTrack::Remote {
                    track: Box::new(track),
                },
            })
            .collect();
        let cover = library
            .playlist_cover(playlist.id)
            .map_err(|error| error.to_string())?;
        playlists.push(SavedPlaylist {
            name: playlist.name,
            entries,
            cover,
        });
    }
    let mut liked = Vec::new();
    for track in library.liked().map_err(|error| error.to_string())? {
        liked.push(SavedLiked {
            id: track.id.0.to_string(),
            kind: "local".into(),
            title: track.title.clone(),
            artist: track.artists.first().cloned().unwrap_or_default(),
            album: track.album.clone(),
        });
    }
    let mut history = Vec::new();
    for entry in library.history(200).map_err(|error| error.to_string())? {
        let t = &entry.track;
        history.push(SavedHistory {
            id: t.id.0.to_string(),
            kind: "local".into(),
            title: t.title.clone(),
            artist: t.artists.first().cloned().unwrap_or_default(),
            album: t.album.clone(),
            played_at: entry.played_at,
            listened_ms: entry.listened_ms as u64,
        });
    }
    let json = serde_json::to_string_pretty(&Backup {
        version: VERSION,
        preferences: config,
        playlists,
        liked,
        history,
    })
    .map_err(|error| error.to_string())?;
    if json.len() > MAX_BYTES {
        return Err("O backup supera o limite de 8 MiB.".into());
    }
    Ok(json)
}

pub(crate) struct Restored {
    pub config: Config,
    pub playlists: usize,
    pub liked: usize,
    pub history: usize,
    pub missing: usize,
}

fn parse(library: &Library, root: &Path, json: &str) -> Result<(Config, Vec<kazzu_library::RestoredPlaylist>, usize, usize, usize), String> {
    if json.len() > MAX_BYTES {
        return Err("O backup supera o limite de 8 MiB.".into());
    }
    let backup: Backup = serde_json::from_str(json).map_err(|_| "Arquivo de backup inválido.")?;
    if backup.version != VERSION {
        return Err("Versão do backup não suportada.".into());
    }

    let mut relative = HashMap::<PathBuf, TrackId>::new();
    let mut metadata = HashMap::<LocalMetadata, Option<TrackId>>::new();
    for track in library.tracks().map_err(|error| error.to_string())? {
        if let Ok(path) = track.path.strip_prefix(root) {
            relative.insert(path.to_owned(), track.id);
        }
        metadata
            .entry(LocalMetadata::from(&track))
            .and_modify(|id| *id = None)
            .or_insert(Some(track.id));
    }

    let mut missing = 0;
    let mut playlists = Vec::new();
    for playlist in backup.playlists {
        let name = playlist.name.trim().to_owned();
        if name.is_empty() || name.len() > 4096 {
            return Err("O backup contém um nome de playlist inválido.".into());
        }
        let entries = playlist
            .entries
            .into_iter()
            .filter_map(|item| match item {
                SavedTrack::Local {
                    relative: path,
                    metadata: tags,
                } => {
                    let track = path
                        .and_then(|path| relative.get(&path).copied())
                        .or_else(|| metadata.get(&tags).copied().flatten());
                    if track.is_none() {
                        missing += 1;
                    }
                    track.map(kazzu_library::RestorePlaylistEntry::Local)
                }
                SavedTrack::Remote { mut track } => {
                    if matches!(&track.source, kazzu_core::TrackSource::Remote { .. }) {
                        track.cover = None;
                        Some(kazzu_library::RestorePlaylistEntry::Remote(track))
                    } else {
                        missing += 1;
                        None
                    }
                }
            })
            .collect::<Vec<_>>();
        let cover = playlist
            .cover
            .map(|bytes| crate::playlist_cover::encode(&bytes))
            .transpose()?;
        playlists.push(kazzu_library::RestoredPlaylist {
            name,
            entries,
            cover,
        });
    }
    let liked_count = backup.liked.len();
    let history_count = backup.history.len();
    Ok((backup.preferences.sanitized(), playlists, liked_count, history_count, missing))
}

pub(crate) fn restore(library: &mut Library, root: &Path, json: &str) -> Result<Restored, String> {
    let (config, playlists, liked, history, missing) = parse(library, root, json)?;
    let count = playlists.len();
    library
        .restore_playlists(&playlists)
        .map_err(|error| error.to_string())?;
    Ok(Restored {
        config,
        playlists: count,
        liked,
        history,
        missing,
    })
}

pub(crate) fn sync_playlists(library: &mut Library, root: &Path, json: &str) -> Result<usize, String> {
    let (_, playlists, _, _, _) = parse(library, root, json)?;
    library
        .restore_playlists(&playlists)
        .map_err(|error| error.to_string())
}
