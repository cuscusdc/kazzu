//! Matches an M3U to tracks already indexed in the library.
//! Paths from documents are never opened. Ambiguous basenames stay unresolved.

use std::collections::HashMap;
use std::path::Path;

use kazzu_library::{Track, TrackId};

fn portable_name(filename: &str) -> Option<String> {
    let (stem, extension) = filename.rsplit_once('.')?;
    let clean: String = stem
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || " ._()-".contains(character) {
                character
            } else {
                '_'
            }
        })
        .collect();
    let clean = clean.trim();
    let stem = if clean.is_empty() {
        "audio".into()
    } else {
        clean.chars().take(48).collect::<String>()
    };
    Some(format!(
        "{}.{}",
        stem.to_lowercase(),
        extension.to_ascii_lowercase()
    ))
}

pub(crate) fn resolve(contents: &str, tracks: Vec<Track>) -> (Vec<TrackId>, usize) {
    let mut paths = HashMap::new();
    let mut names: HashMap<String, Option<TrackId>> = HashMap::new();
    for track in tracks {
        paths.insert(track.path.to_string_lossy().into_owned(), track.id);
        let Some(filename) = track.path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let name = filename
            .split_once('-')
            .filter(|(hash, _)| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .map_or(filename, |(_, original)| original);
        names
            .entry(name.to_lowercase())
            .and_modify(|id| *id = None)
            .or_insert(Some(track.id));
    }

    let mut matched = Vec::new();
    let mut skipped = 0;
    for line in contents
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
    {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.contains("://") {
            skipped += 1;
            continue;
        }
        let path = line.replace('\\', "/");
        let id = paths.get(&path).copied().or_else(|| {
            let name = Path::new(&path).file_name()?.to_str()?;
            match names.get(&name.to_lowercase()) {
                Some(id) => *id,
                None => portable_name(name).and_then(|name| names.get(&name).copied().flatten()),
            }
        });
        match id {
            Some(id) => matched.push(id),
            None => skipped += 1,
        }
    }
    (matched, skipped)
}
