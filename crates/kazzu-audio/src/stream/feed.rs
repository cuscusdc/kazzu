//! Remote decode worker: opens an [`HttpSource`], probes and decodes it on
//! its own thread and hands PCM to the engine through a bounded channel.
//! The engine only ever calls `try_recv`, so it never blocks on the network.

use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, TryRecvError};
use kazzu_core::{AudioFormat, Codec, Error};
use symphonia::core::formats::probe::Hint;

use super::NetContext;
use super::cache::CacheWriter;
use super::http::{HttpSource, Link, NetFailure, Request, spawn_download};
use crate::decoder::{SourceSpec, TrackDecoder};

/// Decoded chunks buffered between the worker and the engine.
const CHANNEL_CHUNKS: usize = 32;
/// Frames per chunk sent to the engine (about 85 ms at 48 kHz).
const CHUNK_FRAMES: usize = 4096;

/// Why a remote feed stopped.
#[derive(Debug)]
pub(crate) enum FeedFailure {
    /// The URL expired (HTTP 401/403/410): needs a fresh `resolve_stream`.
    Expired,
    /// Network down after every retry: may work later from the same URL.
    Network(String),
    /// Anything else (bad data, unsupported codec, HTTP 404): skip the item.
    Fatal(Error),
}

impl FeedFailure {
    pub fn to_error(&self) -> Error {
        match self {
            FeedFailure::Expired => Error::Network("stream URL expired".into()),
            FeedFailure::Network(msg) => Error::Network(msg.clone()),
            FeedFailure::Fatal(e) => Error::other(e.to_string()),
        }
    }
}

/// What the stream turned out to be.
#[derive(Debug, Clone)]
pub(crate) struct OpenInfo {
    pub spec: SourceSpec,
    pub duration: Option<Duration>,
    pub format: AudioFormat,
}

enum Msg {
    Opened(OpenInfo),
    Pcm(Vec<f32>),
    End,
    Failed(FeedFailure),
}

pub(crate) enum OpenPoll {
    Pending,
    Ready,
    Failed(FeedFailure),
}

pub(crate) enum Pull {
    /// Samples appended.
    Data,
    /// Nothing decoded yet (network slower than playback).
    Pending,
    End,
    Failed(FeedFailure),
}

/// Engine-side handle of one remote stream. Dropping it cancels the
/// download and stops the worker.
pub(crate) struct RemoteFeed {
    rx: Receiver<Msg>,
    link: Arc<Link>,
    info: Option<OpenInfo>,
    done: bool,
    /// Decoded to the end: on drop, let the download finish the disk cache.
    ended: bool,
}

/// Parameters of a remote open.
pub(crate) struct RemoteSpec {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub format: Option<AudioFormat>,
    /// Start position (seek after probing).
    pub position: Duration,
    /// Duration from the track metadata (fragmented MP4 under-reports).
    pub duration_hint: Option<Duration>,
    pub cache_key: Option<String>,
}

impl RemoteFeed {
    pub fn spawn(ctx: &Arc<NetContext>, spec: RemoteSpec) -> Self {
        Self::spawn_with(ctx, move |_| Ok(spec))
    }

    pub fn spawn_with(
        ctx: &Arc<NetContext>,
        resolve: impl FnOnce(&Link) -> Result<RemoteSpec, FeedFailure> + Send + 'static,
    ) -> Self {
        let (tx, rx) = crossbeam_channel::bounded(CHANNEL_CHUNKS);
        let link = Link::new();
        let feed = Self {
            rx,
            link: Arc::clone(&link),
            info: None,
            done: false,
            ended: false,
        };
        let ctx = Arc::clone(ctx);
        let spawned = std::thread::Builder::new()
            .name("kazzu-stream".into())
            .spawn(move || {
                if link.is_cancelled() {
                    return;
                }
                match resolve(&link) {
                    Ok(spec) if !link.is_cancelled() => worker(ctx, link, spec, tx),
                    Ok(_) => {}
                    Err(error) => { let _ = tx.send(Msg::Failed(error)); }
                }
            });
        if let Err(e) = spawned {
            // The receiver reports a disconnected channel as a failure.
            log::warn!("cannot start stream worker: {e}");
        }
        feed
    }

    pub fn info(&self) -> Option<&OpenInfo> {
        self.info.as_ref()
    }

    /// (bytes received, HTTP requests made).
    #[allow(dead_code)]
    pub fn stats(&self) -> (u64, u32) {
        self.link.stats()
    }

    /// Checks whether the stream has been opened (probe + seek done).
    pub fn poll_open(&mut self) -> OpenPoll {
        if self.info.is_some() {
            return OpenPoll::Ready;
        }
        if self.done {
            return OpenPoll::Failed(FeedFailure::Fatal(Error::other("stream already failed")));
        }
        match self.rx.try_recv() {
            Ok(Msg::Opened(info)) => {
                self.info = Some(info);
                OpenPoll::Ready
            }
            Ok(Msg::Failed(f)) => {
                self.done = true;
                OpenPoll::Failed(f)
            }
            Ok(Msg::Pcm(_) | Msg::End) => {
                self.done = true;
                OpenPoll::Failed(FeedFailure::Fatal(Error::other("stream worker protocol error")))
            }
            Err(TryRecvError::Empty) => OpenPoll::Pending,
            Err(TryRecvError::Disconnected) => {
                self.done = true;
                OpenPoll::Failed(FeedFailure::Fatal(Error::other("stream worker stopped")))
            }
        }
    }

    /// Takes the next decoded chunk (only after `poll_open` was `Ready`).
    pub fn pull(&mut self, out: &mut Vec<f32>) -> Pull {
        if self.done {
            return Pull::End;
        }
        match self.rx.try_recv() {
            Ok(Msg::Pcm(pcm)) => {
                out.extend_from_slice(&pcm);
                Pull::Data
            }
            Ok(Msg::End) => {
                self.done = true;
                self.ended = true;
                Pull::End
            }
            Ok(Msg::Failed(f)) => {
                self.done = true;
                Pull::Failed(f)
            }
            Ok(Msg::Opened(_)) => Pull::Pending,
            Err(TryRecvError::Empty) => Pull::Pending,
            Err(TryRecvError::Disconnected) => {
                self.done = true;
                Pull::Failed(FeedFailure::Fatal(Error::other("stream worker stopped")))
            }
        }
    }
}

impl Drop for RemoteFeed {
    fn drop(&mut self) {
        if self.ended {
            // Played to the end: holes left by seeks are still worth
            // filling for the cache (bounded by the file size).
            self.link.detach();
        } else {
            self.link.cancel();
        }
    }
}

fn classify(link: &Link, error: Error) -> FeedFailure {
    match link.failure() {
        Some(NetFailure::Expired(_)) => FeedFailure::Expired,
        Some(NetFailure::Network(msg)) => FeedFailure::Network(msg),
        Some(f @ NetFailure::Http(_)) => FeedFailure::Fatal(Error::Network(f.to_string())),
        None => FeedFailure::Fatal(error),
    }
}

fn worker(ctx: Arc<NetContext>, link: Arc<Link>, spec: RemoteSpec, tx: crossbeam_channel::Sender<Msg>) {
    let cache: Option<CacheWriter> = spec
        .cache_key
        .clone()
        .and_then(|key| ctx.cache.as_ref().and_then(|c| c.writer(key)));
    let request = Request {
        url: spec.url.clone(),
        headers: spec.headers.clone(),
    };
    if let Err(e) = spawn_download(Arc::clone(&ctx), Arc::clone(&link), request, cache) {
        let _ = tx.send(Msg::Failed(FeedFailure::Fatal(Error::Io(e))));
        return;
    }

    let source = match HttpSource::connect(Arc::clone(&link)) {
        Ok(s) => s,
        Err(f) => {
            let failure = classify(&link, Error::Network(f.to_string()));
            let _ = tx.send(Msg::Failed(failure));
            return;
        }
    };
    let hint = make_hint(&spec, link.content_type().as_deref());
    let mut decoder = match TrackDecoder::open_media(Box::new(source), &hint, &redact(&spec.url)) {
        Ok(d) => d,
        Err(e) => {
            let _ = tx.send(Msg::Failed(classify(&link, e)));
            return;
        }
    };
    decoder.reconcile_duration(spec.duration_hint);
    if !spec.position.is_zero()
        && let Err(e) = decoder.seek(spec.position)
    {
        let _ = tx.send(Msg::Failed(classify(&link, e)));
        return;
    }
    let info = OpenInfo {
        spec: decoder.spec(),
        duration: decoder.duration(),
        format: decoder.audio_format().clone(),
    };
    let channels = usize::from(info.spec.channels.max(1));
    if tx.send(Msg::Opened(info)).is_err() {
        return;
    }

    let mut pcm = Vec::with_capacity(CHUNK_FRAMES * channels * 2);
    loop {
        match decoder.decode_next(&mut pcm) {
            Ok(true) => {
                if pcm.len() >= CHUNK_FRAMES * channels {
                    let chunk = std::mem::replace(&mut pcm, Vec::with_capacity(CHUNK_FRAMES * channels * 2));
                    if tx.send(Msg::Pcm(chunk)).is_err() {
                        return;
                    }
                }
            }
            Ok(false) => {
                if !pcm.is_empty() && tx.send(Msg::Pcm(pcm)).is_err() {
                    return;
                }
                let _ = tx.send(Msg::End);
                return;
            }
            Err(e) => {
                // Hand over what was decoded before the failure.
                if !pcm.is_empty() && tx.send(Msg::Pcm(std::mem::take(&mut pcm))).is_err() {
                    return;
                }
                let _ = tx.send(Msg::Failed(classify(&link, e)));
                return;
            }
        }
    }
}

/// Probe hint from the provider's format, the Content-Type and the URL.
/// symphonia also sniffs the bytes, so this only orders the guesses.
fn make_hint(spec: &RemoteSpec, content_type: Option<&str>) -> Hint {
    let mut hint = Hint::new();
    if let Some(ct) = content_type {
        let mime = ct.split(';').next().unwrap_or(ct).trim();
        hint.mime_type(mime);
        if let Some(ext) = ext_for_mime(mime) {
            hint.with_extension(ext);
            return hint;
        }
    }
    if let Some(ext) = url_extension(&spec.url) {
        hint.with_extension(&ext);
        return hint;
    }
    if let Some(format) = &spec.format {
        let ext = match format.codec {
            Codec::Opus => Some("webm"),
            Codec::Aac | Codec::Alac => Some("m4a"),
            Codec::Flac => Some("flac"),
            Codec::Mp3 => Some("mp3"),
            Codec::Vorbis => Some("ogg"),
            Codec::Pcm => Some("wav"),
            _ => None,
        };
        if let Some(ext) = ext {
            hint.with_extension(ext);
        }
    }
    hint
}

fn ext_for_mime(mime: &str) -> Option<&'static str> {
    Some(match mime.to_ascii_lowercase().as_str() {
        "audio/webm" | "video/webm" => "webm",
        "audio/x-matroska" | "video/x-matroska" => "mka",
        "audio/mp4" | "video/mp4" | "audio/m4a" | "audio/x-m4a" | "audio/aac" => "m4a",
        "audio/ogg" | "application/ogg" | "audio/opus" => "ogg",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        _ => return None,
    })
}

fn url_extension(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let last = path.rsplit('/').next()?;
    let (_, ext) = last.rsplit_once('.')?;
    (1..=4).contains(&ext.len()).then(|| ext.to_ascii_lowercase())
}

/// URL without its query string, for logs and errors (signed URLs carry
/// tokens in the query).
fn redact(url: &str) -> String {
    url.split('?').next().unwrap_or(url).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_helpers() {
        assert_eq!(url_extension("https://h/a/b/song.FLAC?x=1"), Some("flac".into()));
        assert_eq!(url_extension("https://h/videoplayback?expire=1"), None);
        assert_eq!(redact("https://h/v?sig=secret"), "https://h/v");
        assert_eq!(ext_for_mime("audio/webm"), Some("webm"));
    }
}
