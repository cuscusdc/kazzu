//! LRU disk cache of fully downloaded streams (base for offline playback).
//!
//! Entries are plain files named after the track (`<key>.<ext>`), so a
//! cached track opens like a local file, without any network call. While a
//! stream downloads, its bytes are also written at their offsets into
//! `<key>.<n>.part`; once every byte is there the file is renamed into place and
//! the oldest entries are evicted until the cache fits its size limit.
//! Recency = file modification time (refreshed on every hit).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use kazzu_core::{Track, TrackSource};

const PART: &str = "part";

/// Disk cache handle (cheap to share).
#[derive(Debug)]
pub struct StreamCache {
    dir: PathBuf,
    max_bytes: u64,
    /// Serializes renames/evictions between streams.
    lock: Mutex<()>,
}

impl StreamCache {
    /// Opens (creating) the cache directory and removes partial files left
    /// by a previous run.
    pub fn new(dir: impl Into<PathBuf>, max_bytes: u64) -> io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        for entry in fs::read_dir(&dir)?.flatten() {
            if entry.path().extension().is_some_and(|e| e == PART) {
                let _ = fs::remove_file(entry.path());
            }
        }
        Ok(Self {
            dir,
            max_bytes,
            lock: Mutex::new(()),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Stable cache key of a track: its provider and provider id (stream
    /// URLs change on every resolve, so they cannot be the key). `None` for
    /// local tracks.
    pub fn key_for(track: &Track) -> Option<String> {
        match &track.source {
            TrackSource::Remote { provider, id } => Some(file_key(&format!("{provider:?}:{id}"))),
            TrackSource::Local { .. } => None,
        }
    }

    /// Cached file of `track`, if it was downloaded completely before.
    /// Counts as a use (LRU).
    pub fn cached(&self, track: &Track) -> Option<PathBuf> {
        self.lookup(&Self::key_for(track)?)
    }

    pub(crate) fn lookup(&self, key: &str) -> Option<PathBuf> {
        let prefix = format!("{key}.");
        let path = fs::read_dir(&self.dir).ok()?.flatten().map(|e| e.path()).find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix))
                && p.extension().is_none_or(|e| e != PART)
        })?;
        if let Ok(f) = File::options().append(true).open(&path) {
            let _ = f.set_modified(SystemTime::now());
        }
        Some(path)
    }

    /// Bytes used by complete entries.
    pub fn size(&self) -> u64 {
        self.entries().iter().map(|e| e.1).sum()
    }

    /// Lists completed cache entries without updating their recency.
    pub fn complete_keys(&self) -> std::collections::HashSet<String> {
        self.entries().into_iter().filter_map(|(path, _, _)| {
            Some(path.file_name()?.to_str()?.split_once('.')?.0.to_owned())
        }).collect()
    }

    /// Evicts completed copies of one track. Active partial files are retained.
    pub fn remove_track(&self, track: &Track) -> io::Result<bool> {
        let Some(key) = Self::key_for(track) else { return Ok(false) };
        let _guard = self.lock.lock().unwrap_or_else(|error| error.into_inner());
        let mut removed = false;
        for (path, _, _) in self.entries() {
            if path.file_name().and_then(|name| name.to_str()).and_then(|name| name.split_once('.'))
                .is_some_and(|(stored, _)| stored == key)
            {
                match fs::remove_file(path) {
                    Ok(()) => removed = true,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(removed)
    }

    /// Deletes every complete entry.
    pub fn clear(&self) -> io::Result<()> {
        self.clear_and_count().map(|_| ())
    }

    /// Clears completed streams only and returns the number of bytes removed.
    pub fn clear_and_count(&self) -> io::Result<u64> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut removed = 0u64;
        for (path, bytes, _) in self.entries() {
            match fs::remove_file(path) {
                Ok(()) => removed = removed.saturating_add(bytes),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(removed)
    }

    fn entries(&self) -> Vec<(PathBuf, u64, SystemTime)> {
        let Ok(dir) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        dir.flatten()
            .filter(|e| e.path().extension().is_none_or(|x| x != PART))
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                meta.is_file().then(|| {
                    (e.path(), meta.len(), meta.modified().unwrap_or(SystemTime::UNIX_EPOCH))
                })
            })
            .collect()
    }

    fn evict(&self, keep: &Path) {
        let mut entries = self.entries();
        let mut total: u64 = entries.iter().map(|e| e.1).sum();
        entries.sort_by_key(|e| e.2);
        for (path, len, _) in entries {
            if total <= self.max_bytes {
                break;
            }
            if path == keep {
                continue;
            }
            if fs::remove_file(&path).is_ok() {
                total -= len;
            }
        }
    }

    pub(crate) fn writer(self: &Arc<Self>, key: String) -> Option<CacheWriter> {
        // Unique per writer: a stream being torn down (seek, skip) may still
        // own an older partial file of the same track.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let part = self.dir.join(format!("{key}.{}-{n}.{PART}", std::process::id()));
        let file = OpenOptions::new().write(true).create_new(true).open(&part).ok()?;
        Some(CacheWriter {
            cache: Arc::clone(self),
            key,
            part,
            file: Some(file),
            len: None,
            covered: Vec::new(),
            done: false,
        })
    }
}

/// Writes one stream's bytes into its `.part` file.
pub(crate) struct CacheWriter {
    cache: Arc<StreamCache>,
    key: String,
    part: PathBuf,
    file: Option<File>,
    len: Option<u64>,
    /// Sorted, merged byte ranges written so far.
    covered: Vec<(u64, u64)>,
    done: bool,
}

impl CacheWriter {
    pub fn set_len(&mut self, len: u64) {
        if self.len.is_some() {
            return;
        }
        if len > self.cache.max_bytes / 2 {
            // Too big to be worth caching (it would evict everything else).
            self.file = None;
        }
        self.len = Some(len);
    }

    pub fn write(&mut self, pos: u64, data: &[u8]) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        let ok = file.seek(SeekFrom::Start(pos)).and_then(|_| file.write_all(data));
        if let Err(e) = ok {
            log::debug!("stream cache write failed, entry dropped: {e}");
            self.file = None;
            return;
        }
        add_range(&mut self.covered, pos, pos + data.len() as u64);
    }

    /// First byte not written yet (holes left by seeks), if any.
    pub fn first_gap(&self, len: u64) -> Option<u64> {
        self.file.as_ref()?;
        match self.covered.first() {
            Some(&(0, end)) if end >= len => None,
            Some(&(0, end)) => Some(end),
            _ => Some(0),
        }
    }

    pub fn is_complete(&self) -> bool {
        self.file.is_some() && self.len.is_some_and(|l| l > 0 && self.first_gap(l).is_none())
    }

    /// Moves a complete file into the cache.
    pub fn finish(mut self, content_type: Option<&str>) {
        if !self.is_complete() {
            return;
        }
        if let Some(file) = self.file.take()
            && file.sync_all().is_err()
        {
            return;
        }
        let path = self.cache.dir.join(format!("{}.{}", self.key, extension_for(content_type)));
        let _guard = self.cache.lock.lock();
        if fs::rename(&self.part, &path).is_ok() {
            self.done = true;
            log::debug!("stream cached: {}", path.display());
            self.cache.evict(&path);
        }
    }
}

impl Drop for CacheWriter {
    fn drop(&mut self) {
        if !self.done {
            self.file = None;
            let _ = fs::remove_file(&self.part);
        }
    }
}

fn add_range(ranges: &mut Vec<(u64, u64)>, start: u64, end: u64) {
    if start >= end {
        return;
    }
    let i = ranges.partition_point(|r| r.1 < start);
    let mut s = start;
    let mut e = end;
    let mut j = i;
    while j < ranges.len() && ranges[j].0 <= e {
        s = s.min(ranges[j].0);
        e = e.max(ranges[j].1);
        j += 1;
    }
    ranges.splice(i..j, [(s, e)]);
}

/// File extension from the response Content-Type (symphonia also probes
/// the content, the extension is only a hint).
fn extension_for(content_type: Option<&str>) -> &'static str {
    let mime = content_type
        .and_then(|c| c.split(';').next())
        .map(|m| m.trim().to_ascii_lowercase())
        .unwrap_or_default();
    match mime.as_str() {
        "audio/webm" | "video/webm" => "webm",
        "audio/mp4" | "video/mp4" | "audio/x-m4a" | "audio/m4a" | "audio/aac" => "m4a",
        "audio/ogg" | "application/ogg" | "audio/opus" => "ogg",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/x-matroska" | "video/x-matroska" => "mka",
        _ => "bin",
    }
}

/// Filesystem-safe, stable name for a cache key: a readable prefix plus a
/// 64-bit FNV-1a hash of the whole key.
fn file_key(key: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let readable: String = key
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .take(40)
        .collect();
    format!("{readable}-{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_merge() {
        let mut r = Vec::new();
        add_range(&mut r, 10, 20);
        add_range(&mut r, 30, 40);
        add_range(&mut r, 0, 5);
        assert_eq!(r, vec![(0, 5), (10, 20), (30, 40)]);
        add_range(&mut r, 5, 30);
        assert_eq!(r, vec![(0, 40)]);
        add_range(&mut r, 35, 50);
        assert_eq!(r, vec![(0, 50)]);
    }

    #[test]
    fn keys_are_stable_and_safe() {
        let a = file_key("YtMusic:abc/../x?y");
        assert_eq!(a, file_key("YtMusic:abc/../x?y"));
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
        assert_ne!(a, file_key("YtMusic:abc/../x?z"));
        assert_eq!(extension_for(Some("audio/webm; codecs=\"opus\"")), "webm");
        assert_eq!(extension_for(Some("audio/mp4")), "m4a");
    }
}
