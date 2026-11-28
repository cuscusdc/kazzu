//! The playback engine: one control/decode thread plus the device callback.
//!
//! ```text
//!  UI ──PlaybackCommand──▶ engine thread ──f32 frames──▶ SPSC ring ──▶ device callback
//!     ◀──PlaybackEvent───  (queue, decode,              (lock-free)    (Renderer: volume,
//!                           resample?, DSP)                              format conversion)
//! ```
//!
//! Gapless: when a track's decoder reaches EOF the next one is opened at
//! once and its samples are appended to the same ring, so the device never
//! sees a gap. Track boundaries are tracked as frame counters ("segments")
//! and compared to the number of frames the callback has consumed, which
//! also yields the playback position.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TryRecvError, TrySendError};
use kazzu_core::{
    AudioFormat, Error, OutputInfo, PlaybackCommand, PlaybackEvent, PlaybackState, PlaybackStatus,
    QueueItem, QueueItemId, RepeatMode, Result, StreamSource,
};
use ringbuf::traits::{Observer, Producer, Split};
use ringbuf::{HeapProd, HeapRb};

use crate::cpal_sink::CpalSink;
use crate::decoder::{SourceSpec, TrackDecoder, append_remapped, frames_to_duration};
use crate::dsp::{Bypass, Dsp};
use crate::output::{AudioSink, OutputConfig, OutputRequest, Renderer, Shared};
use crate::queue::Queue;
use crate::resample::StreamResampler;
use crate::stream::{
    FeedFailure, NetContext, OpenPoll, Pull, RemoteFeed, RemoteSpec, StreamCache, StreamConfig,
    StreamEvent,
};

/// Wake-up period of the engine thread when it has nothing to decode.
const TICK: Duration = Duration::from_millis(5);
/// "Previous" restarts the current track when past this position.
const PREVIOUS_RESTART: Duration = Duration::from_secs(3);
/// Max packets decoded per loop iteration (keeps commands responsive).
const PUMP_BUDGET: usize = 32;
/// Max events queued before dropping (keeps the audio thread never blocked;
/// the UI drains events on its own loop, so a larger buffer only reduces drops).
const EVENT_CAPACITY: usize = 16384;
/// Max stream events queued before dropping.
const STREAM_EVENT_CAPACITY: usize = 1024;
/// Max time to wait for the device callback to acknowledge a flush.
const FLUSH_WAIT: Duration = Duration::from_millis(500);
/// Network buffering: pause the device when the ring falls below this
/// fraction while a remote source has nothing decoded, and resume above
/// `RESUME_FILL` (or when the source ends).
const LOW_FILL: f64 = 0.2;
const RESUME_FILL: f64 = 0.6;

/// Engine tuning.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Initial linear volume (1.0 = unity, bit-perfect).
    pub volume: f32,
    /// Length of the ring buffer between decoder and device.
    pub buffer: Duration,
    /// Interval of [`PlaybackEvent::Position`] while playing.
    pub position_interval: Duration,
    /// HTTP streaming ([`StreamSource::Url`]): timeouts, retries, read-ahead,
    /// bandwidth cap, disk cache.
    pub stream: StreamConfig,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            volume: 1.0,
            buffer: Duration::from_millis(500),
            position_interval: Duration::from_millis(100),
            stream: StreamConfig::default(),
        }
    }
}

/// Entry point. See [`Engine::spawn`] and [`Engine::builder`].
pub struct Engine;

impl Engine {
    /// Spawns an engine on the default sound card (cpal) with default
    /// settings. The device is opened lazily, on first playback.
    pub fn spawn() -> Result<EngineHandle> {
        Self::builder().spawn()
    }

    pub fn builder() -> EngineBuilder {
        EngineBuilder::default()
    }
}

type ResolveStream = Arc<StreamResolver>;

struct StreamResolver {
    resolve: Box<dyn Fn(&kazzu_core::Track) -> Result<StreamSource> + Send + Sync>,
    permits: Receiver<()>,
    release: Sender<()>,
}

impl StreamResolver {
    fn new(resolve: impl Fn(&kazzu_core::Track) -> Result<StreamSource> + Send + Sync + 'static) -> Self {
        let (release, permits) = crossbeam_channel::bounded(2);
        let _ = release.try_send(());
        let _ = release.try_send(());
        Self { resolve: Box::new(resolve), permits, release }
    }

    fn open(&self, track: &kazzu_core::Track, cancelled: impl Fn() -> bool) -> Result<StreamSource> {
        while !cancelled() {
            match self.permits.recv_timeout(Duration::from_millis(50)) {
                Ok(()) => {
                    let result = if cancelled() {
                        Err(Error::other("stream open cancelled"))
                    } else {
                        (self.resolve)(track)
                    };
                    let _ = self.release.try_send(());
                    return result;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        Err(Error::other("stream open cancelled"))
    }
}

/// Configures and spawns an engine.
#[derive(Default)]
pub struct EngineBuilder {
    sink: Option<Box<dyn AudioSink>>,
    dsp: Option<Box<dyn Dsp>>,
    resolver: Option<ResolveStream>,
    config: EngineConfig,
}

impl EngineBuilder {
    /// Output to use (default: [`CpalSink`] on the default device).
    pub fn sink(mut self, sink: impl AudioSink + 'static) -> Self {
        self.sink = Some(Box::new(sink));
        self
    }

    /// DSP stage (default: [`Bypass`]).
    pub fn dsp(mut self, dsp: impl Dsp + 'static) -> Self {
        self.dsp = Some(Box::new(dsp));
        self
    }

    pub fn config(mut self, config: EngineConfig) -> Self {
        self.config = config;
        self
    }

    /// Resolve deferred queue entries on the stream worker, outside the audio loop.
    pub fn resolve_streams(
        mut self,
        resolve: impl Fn(&kazzu_core::Track) -> Result<StreamSource> + Send + Sync + 'static,
    ) -> Self {
        self.resolver = Some(Arc::new(StreamResolver::new(resolve)));
        self
    }

    pub fn spawn(self) -> Result<EngineHandle> {
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        let (ev_tx, ev_rx) = crossbeam_channel::bounded(EVENT_CAPACITY);
        let (stream_tx, stream_rx) = crossbeam_channel::bounded(STREAM_EVENT_CAPACITY);
        let sink = self.sink.unwrap_or_else(|| Box::new(CpalSink::new()));
        let dsp = self.dsp.unwrap_or_else(|| Box::new(Bypass));
        let config = self.config;
        let resolver = self.resolver;
        // Building the agent does no network I/O.
        let net = NetContext::new(config.stream.clone());
        let cache = net.cache.clone();
        let channels = Channels {
            rx: cmd_rx,
            events: ev_tx,
            stream_events: stream_tx,
        };
        let thread = std::thread::Builder::new()
            .name("kazzu-engine".into())
            .spawn(move || {
                let mut core = Core::new(channels, sink, dsp, config, net);
                core.resolver = resolver;
                core.run();
            })?;
        Ok(EngineHandle {
            tx: cmd_tx,
            events: ev_rx,
            stream_events: stream_rx,
            cache,
            thread: Some(thread),
            owner: true,
        })
    }
}

enum Msg {
    Command(PlaybackCommand),
    LoadAt { item: Box<QueueItem>, position: Duration, playing: bool },
    SetDsp(Box<dyn Dsp>),
    ReplaceStream(QueueItemId, StreamSource),
    SelectOutput(Option<String>),
    SleepTimer(Option<Duration>),
    Crossfade(Duration),
    State(Sender<PlaybackState>),
    Shutdown,
}

/// Handle to a running engine. Only the owner stops the thread when dropped.
pub struct EngineHandle {
    tx: Sender<Msg>,
    events: Receiver<PlaybackEvent>,
    stream_events: Receiver<StreamEvent>,
    cache: Option<Arc<StreamCache>>,
    thread: Option<JoinHandle<()>>,
    owner: bool,
}

impl EngineHandle {
    /// Sends any command.
    pub fn send(&self, command: PlaybackCommand) {
        let _ = self.tx.send(Msg::Command(command));
    }

    /// Event stream. The receiver can be cloned; events are dropped (not
    /// queued forever) if nobody reads them.
    pub fn events(&self) -> &Receiver<PlaybackEvent> {
        &self.events
    }

    /// Streaming notifications that need an answer (expired URL, network
    /// failure). See [`StreamEvent`].
    pub fn stream_events(&self) -> &Receiver<StreamEvent> {
        &self.stream_events
    }

    /// Gives an item a new stream, typically a freshly resolved URL after
    /// [`StreamEvent::Expired`]. A stalled item resumes where it stopped
    /// (gapless when the audio before that point is still buffered).
    pub fn replace_stream(&self, id: QueueItemId, stream: StreamSource) {
        let _ = self.tx.send(Msg::ReplaceStream(id, stream));
    }

    /// Switches to a CPAL output, preserving playback position.
    pub fn select_output(&self, id: Option<String>) {
        let _ = self.tx.send(Msg::SelectOutput(id));
    }

    /// Pauses playback when the requested time elapses.
    pub fn set_sleep_timer(&self, duration: Option<Duration>) {
        let _ = self.tx.send(Msg::SleepTimer(duration));
    }

    /// Enables a linear PCM overlap for compatible local files.
    pub fn set_crossfade(&self, duration: Duration) {
        let _ = self.tx.send(Msg::Crossfade(duration.min(Duration::from_secs(8))));
    }

    /// The disk cache of downloaded streams, if configured
    /// (`StreamConfig::cache`).
    pub fn cache(&self) -> Option<&Arc<StreamCache>> {
        self.cache.as_ref()
    }

    /// Snapshot of the full player state.
    pub fn state(&self) -> Option<PlaybackState> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.tx.send(Msg::State(tx)).ok()?;
        rx.recv_timeout(Duration::from_secs(2)).ok()
    }

    /// Replaces the DSP stage (equalizer etc.).
    pub fn set_dsp(&self, dsp: impl Dsp + 'static) {
        let _ = self.tx.send(Msg::SetDsp(Box::new(dsp)));
    }

    pub fn load(&self, items: Vec<QueueItem>, start: usize) {
        self.send(PlaybackCommand::Load { items, start });
    }

    /// Start an individual queue entry at a peer's playback position.
    pub fn load_at(&self, item: QueueItem, position: Duration, playing: bool) {
        let _ = self.tx.send(Msg::LoadAt { item: Box::new(item), position, playing });
    }

    pub fn enqueue(&self, items: Vec<QueueItem>) {
        self.send(PlaybackCommand::Enqueue { items });
    }

    pub fn play_next(&self, items: Vec<QueueItem>) {
        self.send(PlaybackCommand::PlayNext { items });
    }

    pub fn play(&self) {
        self.send(PlaybackCommand::Play);
    }

    pub fn pause(&self) {
        self.send(PlaybackCommand::Pause);
    }

    pub fn toggle_pause(&self) {
        self.send(PlaybackCommand::TogglePause);
    }

    pub fn stop(&self) {
        self.send(PlaybackCommand::Stop);
    }

    pub fn seek(&self, position: Duration) {
        self.send(PlaybackCommand::Seek { position });
    }

    pub fn set_volume(&self, volume: f32) {
        self.send(PlaybackCommand::SetVolume { volume });
    }

    pub fn next(&self) {
        self.send(PlaybackCommand::Next);
    }

    pub fn previous(&self) {
        self.send(PlaybackCommand::Previous);
    }

    pub fn jump_to(&self, id: QueueItemId) {
        self.send(PlaybackCommand::JumpTo { id });
    }

    pub fn remove(&self, id: QueueItemId) {
        self.send(PlaybackCommand::Remove { id });
    }

    pub fn move_before(&self, id: QueueItemId, before: QueueItemId) {
        self.send(PlaybackCommand::MoveBefore { id, before });
    }

    pub fn clear(&self) {
        self.send(PlaybackCommand::Clear);
    }

    pub fn set_repeat(&self, mode: RepeatMode) {
        self.send(PlaybackCommand::SetRepeat { mode });
    }

    pub fn set_shuffle(&self, enabled: bool) {
        self.send(PlaybackCommand::SetShuffle { enabled });
    }

    /// Stops the engine and waits for its thread.
    pub fn shutdown(mut self) {
        self.stop_thread();
    }

    fn stop_thread(&mut self) {
        if !self.owner {
            return;
        }
        let _ = self.tx.send(Msg::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Clone for EngineHandle {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            events: self.events.clone(),
            stream_events: self.stream_events.clone(),
            cache: self.cache.clone(),
            thread: None,
            owner: false,
        }
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        self.stop_thread();
    }
}

// ---------------------------------------------------------------------------
// Engine thread internals
// ---------------------------------------------------------------------------

/// The open output stream and the producer side of its ring.
struct Output {
    config: OutputConfig,
    /// Source spec the output was opened for.
    request: OutputRequest,
    shared: Arc<Shared>,
    prod: HeapProd<f32>,
    /// Frames pushed into the ring since the stream was opened.
    written: u64,
    flush_gen: u64,
}

impl Output {
    fn flush_pending(&self) -> bool {
        self.shared.flush_ack.load(Ordering::Acquire) != self.flush_gen
    }

    fn played(&self) -> u64 {
        self.shared.frames_played.load(Ordering::Acquire)
    }

    /// Can a source with `spec` be appended without reopening (gapless)?
    fn accepts(&self, spec: SourceSpec) -> bool {
        let same_source = self.request.sample_rate == spec.sample_rate
            && self.request.channels == spec.channels
            && self.request.bits_per_sample.unwrap_or(0) >= spec.bits_per_sample.unwrap_or(0);
        let native = self.config.sample_rate == spec.sample_rate
            && channels_compatible(spec.channels, self.config.channels)
            && depth_ok(self.config.sample_format.bits(), spec.bits_per_sample);
        same_source || native
    }
}

fn channels_compatible(source: u16, output: u16) -> bool {
    source == output || (source == 1 && output == 2)
}

/// Whether a device of `device_bits` (None = float) carries a source of
/// `source_bits` without losing precision.
fn depth_ok(device_bits: Option<u32>, source_bits: Option<u32>) -> bool {
    match (device_bits, source_bits) {
        // f32 is exact up to 24-bit integers; lossy sources are float anyway.
        (None, s) => s.is_none_or(|b| b <= 24),
        (Some(_), None) => true,
        (Some(d), Some(s)) => d >= s,
    }
}

/// Where a track's PCM comes from.
enum Feed {
    /// Local file (or cached stream), decoded on the engine thread.
    Local(TrackDecoder),
    /// Remote stream decoded on its worker thread (possibly reconnecting).
    Remote(RemoteFeed),
    /// Stream stopped (expired URL / network down); waits for a resume.
    Dead,
}

impl Feed {
    /// Spec, duration and format of an opened feed.
    fn info(&self) -> Option<(SourceSpec, Option<Duration>, AudioFormat)> {
        match self {
            Feed::Local(d) => Some((d.spec(), d.duration(), d.audio_format().clone())),
            Feed::Remote(r) => r.info().map(|i| (i.spec, i.duration, i.format.clone())),
            Feed::Dead => None,
        }
    }
}

/// Result of opening a queue item.
enum Opened {
    /// Ready to decode now (local file or cache hit; not seeked yet).
    Ready(Feed),
    /// Opening over the network (already seeked to the requested position).
    Waiting(RemoteFeed),
}

/// A track being decoded into the ring.
struct Decoding {
    id: QueueItemId,
    feed: Feed,
    spec: SourceSpec,
    resampler: Option<StreamResampler>,
    out_channels: usize,
    /// Processed samples (output format) not yet in the ring.
    pcm: Vec<f32>,
    pcm_pos: usize,
    raw: Vec<f32>,
    mapped: Vec<f32>,
    eof: bool,
    /// Track position where decoding started (seek target).
    start_offset: Duration,
    /// Source frames decoded since `start_offset`.
    src_frames: u64,
    /// The last fill found no data (remote source behind playback).
    starved: bool,
}

impl Decoding {
    fn new(id: QueueItemId, feed: Feed, spec: SourceSpec, offset: Duration, out: &OutputConfig) -> Result<Self> {
        let resampler = if spec.sample_rate != out.sample_rate {
            Some(
                StreamResampler::new(spec.sample_rate, out.sample_rate, out.channels)
                    .map_err(Error::Output)?,
            )
        } else {
            None
        };
        Ok(Self {
            id,
            feed,
            spec,
            resampler,
            out_channels: usize::from(out.channels),
            pcm: Vec::new(),
            pcm_pos: 0,
            raw: Vec::new(),
            mapped: Vec::new(),
            eof: false,
            start_offset: offset,
            src_frames: 0,
            starved: false,
        })
    }

    fn has_pending(&self) -> bool {
        self.pcm_pos < self.pcm.len()
    }

    /// Track position of the next frame to decode (where a reconnect resumes).
    fn resume_position(&self) -> Duration {
        self.start_offset + frames_to_duration(self.src_frames, self.spec.sample_rate)
    }

    /// Decodes one packet into `pcm` (channel map → resample → DSP).
    /// `Ok(false)` = no data available yet (remote source).
    fn fill(&mut self, dsp: &mut dyn Dsp) -> std::result::Result<bool, FeedFailure> {
        if !self.has_pending() {
            self.pcm.clear();
            self.pcm_pos = 0;
        }
        let start = self.pcm.len();
        self.raw.clear();
        let more = match &mut self.feed {
            Feed::Local(d) => d.decode_next(&mut self.raw).map_err(FeedFailure::Fatal)?,
            Feed::Remote(r) => {
                match r.poll_open() {
                    OpenPoll::Pending => {
                        self.starved = true;
                        return Ok(false);
                    }
                    OpenPoll::Failed(f) => return Err(f),
                    OpenPoll::Ready => {}
                }
                if r.info().is_some_and(|i| i.spec != self.spec) {
                    return Err(FeedFailure::Fatal(Error::Decode(
                        "stream format changed after reconnecting".into(),
                    )));
                }
                match r.pull(&mut self.raw) {
                    Pull::Data => true,
                    Pull::Pending => {
                        self.starved = true;
                        return Ok(false);
                    }
                    Pull::End => false,
                    Pull::Failed(f) => return Err(f),
                }
            }
            Feed::Dead => {
                self.starved = true;
                return Ok(false);
            }
        };
        self.starved = false;

        let src_channels = usize::from(self.spec.channels.max(1));
        self.src_frames += (self.raw.len() / src_channels) as u64;
        let stage: &[f32] = if src_channels == self.out_channels {
            &self.raw
        } else {
            self.mapped.clear();
            append_remapped(&self.raw, src_channels, self.out_channels, &mut self.mapped);
            &self.mapped
        };
        let resampled = match &mut self.resampler {
            Some(rs) => rs.process(stage, &mut self.pcm).and_then(|()| {
                if more { Ok(()) } else { rs.finish(&mut self.pcm) }
            }),
            None => {
                self.pcm.extend_from_slice(stage);
                Ok(())
            }
        };
        resampled.map_err(|e| FeedFailure::Fatal(Error::Output(e)))?;
        if !dsp.is_bypassed() && self.pcm.len() > start {
            dsp.process(&mut self.pcm[start..]);
        }
        if !more {
            self.eof = true;
        }
        Ok(true)
    }

    /// Pushes whole frames into the ring. Returns frames pushed.
    fn push(&mut self, prod: &mut HeapProd<f32>, frames: usize) -> u64 {
        let vacant = prod.vacant_len();
        let room = vacant - vacant % self.out_channels;
        let n = room.min(self.pcm.len() - self.pcm_pos).min(frames.saturating_mul(self.out_channels));
        if n == 0 {
            return 0;
        }
        let pushed = prod.push_slice(&self.pcm[self.pcm_pos..self.pcm_pos + n]);
        self.pcm_pos += pushed;
        (pushed / self.out_channels) as u64
    }
}

/// The following decoder is primed before overlap begins. Keeping its buffered
/// samples preserves the exact outgoing offset when it becomes the current feed.
struct Crossfade {
    from: QueueItemId,
    incoming: Decoding,
    at: u64,
    frames: usize,
    mixed: usize,
    scratch: Vec<f32>,
}

impl Crossfade {
    fn push(&mut self, outgoing: &mut Decoding, out: &mut Output) -> u64 {
        let channels = outgoing.out_channels;
        if out.written < self.at {
            return outgoing.push(&mut out.prod, (self.at - out.written) as usize);
        }
        let remaining_out = (outgoing.pcm.len() - outgoing.pcm_pos) / channels;
        let remaining_in = (self.incoming.pcm.len() - self.incoming.pcm_pos) / channels;
        let frames = remaining_out.min(remaining_in).min(self.frames - self.mixed)
            .min(out.prod.vacant_len() / channels);
        self.scratch.clear();
        for frame in 0..frames {
            let incoming_gain = (self.mixed + frame + 1) as f32 / self.frames as f32;
            for channel in 0..channels {
                let index = frame * channels + channel;
                let outgoing_sample = outgoing.pcm[outgoing.pcm_pos + index];
                let incoming_sample = self.incoming.pcm[self.incoming.pcm_pos + index];
                self.scratch.push(outgoing_sample * (1.0 - incoming_gain) + incoming_sample * incoming_gain);
            }
        }
        let written = out.prod.push_slice(&self.scratch);
        let mixed = written / channels;
        self.mixed += mixed;
        outgoing.pcm_pos += written;
        self.incoming.pcm_pos += written;
        if self.mixed == self.frames {
            // No outgoing samples after the configured tail, including any codec padding.
            outgoing.pcm_pos = outgoing.pcm.len();
            outgoing.eof = true;
        }
        mixed as u64
    }
}

/// Kind of stream stall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StallKind {
    /// Waiting for `replace_stream` with a fresh URL.
    Expired,
    /// Network down; `play()` retries.
    Network,
}

/// An item whose stream stopped and waits to be resumed at `position`.
struct Stall {
    id: QueueItemId,
    position: Duration,
    kind: StallKind,
    since: Instant,
}

/// The current item being opened over the network.
struct Opening {
    id: QueueItemId,
    position: Duration,
    feed: RemoteFeed,
}

/// A track's span inside the ring, in output frames.
struct Segment {
    id: QueueItemId,
    start: u64,
    end: Option<u64>,
    /// Track position at `start` (non-zero after a seek).
    offset: Duration,
    rate: u32,
    duration: Option<Duration>,
    format: AudioFormat,
    source_channels: u16,
    resampled_from: Option<u32>,
}

/// Channels between the handle and the engine thread.
struct Channels {
    rx: Receiver<Msg>,
    events: Sender<PlaybackEvent>,
    stream_events: Sender<StreamEvent>,
}

struct Core {
    sleep_deadline: Option<Instant>,
    crossfade_duration: Duration,
    crossfade: Option<Crossfade>,
    fade_checked: Option<(QueueItemId, QueueItemId)>,
    resolver: Option<ResolveStream>,
    rx: Receiver<Msg>,
    events: Sender<PlaybackEvent>,
    stream_events: Sender<StreamEvent>,
    net: Arc<NetContext>,
    sink: Box<dyn AudioSink>,
    dsp: Box<dyn Dsp>,
    config: EngineConfig,
    queue: Queue,
    status: PlaybackStatus,
    volume: f32,
    out: Option<Output>,
    dec: Option<Decoding>,
    /// Last decoded item when nothing followed it (resumes gapless if the
    /// queue grows before playback drains).
    tail: Option<QueueItemId>,
    /// Next track, opened but waiting for the ring to drain because it
    /// needs a different output format.
    pending_reopen: Option<(QueueItemId, Feed)>,
    segments: VecDeque<Segment>,
    last_position: Instant,
    running: bool,
    /// The user wants sound (Play), as opposed to the visible status, which
    /// can be `Buffering` while waiting for the network.
    want_play: bool,
    /// Device paused because a remote source fell behind.
    underrun: bool,
    /// Current item being opened over the network.
    opening: Option<Opening>,
    /// Next item being opened over the network for the gapless join.
    next_opening: Option<(QueueItemId, RemoteFeed)>,
    /// Next remote item opened early (`StreamConfig::prebuffer`).
    preopen: Option<(QueueItemId, RemoteFeed)>,
    /// Next item found in the disk cache: no pre-open needed (and no need
    /// to look the cache up again on every tick).
    preopen_cached: Option<QueueItemId>,
    stall: Option<Stall>,
    /// Last expiry (item, position): a second one at the same point means
    /// the refreshed URL does not work either.
    last_expired: Option<(QueueItemId, Duration)>,
    /// Consecutive items that failed to open (stops a dead queue looping).
    failed_opens: usize,
}

impl Core {
    fn new(
        channels: Channels,
        sink: Box<dyn AudioSink>,
        dsp: Box<dyn Dsp>,
        config: EngineConfig,
        net: Arc<NetContext>,
    ) -> Self {
        let volume = config.volume.clamp(0.0, 1.0);
        Self {
            resolver: None,
            sleep_deadline: None,
            crossfade_duration: Duration::ZERO,
            crossfade: None,
            fade_checked: None,
            rx: channels.rx,
            events: channels.events,
            stream_events: channels.stream_events,
            net,
            sink,
            dsp,
            config,
            queue: Queue::new(),
            status: PlaybackStatus::Stopped,
            volume,
            out: None,
            dec: None,
            tail: None,
            pending_reopen: None,
            segments: VecDeque::new(),
            last_position: Instant::now(),
            running: true,
            want_play: false,
            underrun: false,
            opening: None,
            next_opening: None,
            preopen: None,
            preopen_cached: None,
            stall: None,
            last_expired: None,
            failed_opens: 0,
        }
    }

    fn run(mut self) {
        while self.running {
            loop {
                match self.rx.try_recv() {
                    Ok(msg) => self.handle(msg),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.running = false;
                        break;
                    }
                }
            }
            if !self.running {
                break;
            }
            let worked = self.pump();
            self.update();
            if !worked {
                let idle = self.status == PlaybackStatus::Stopped
                    && self.out.is_none() && self.opening.is_none() && self.next_opening.is_none();
                let delayed = self.sleep_deadline.map(|deadline| deadline.saturating_duration_since(Instant::now()));
                let received = if idle {
                    match delayed {
                        Some(wait) => self.rx.recv_timeout(wait),
                        None => self.rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                    }
                } else {
                    self.rx.recv_timeout(delayed.map_or(TICK, |wait| TICK.min(wait)))
                };
                match received {
                    Ok(msg) => self.handle(msg),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => self.running = false,
                }
            }
        }
        self.sink.close();
    }

    fn emit(&self, event: PlaybackEvent) {
        match self.events.try_send(event) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => log::trace!("event channel full, event dropped"),
        }
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Command(cmd) => self.command(cmd),
            Msg::LoadAt { item, position, playing } => {
                self.reset_pipeline();
                let id = item.id;
                self.queue.load(vec![*item], 0);
                self.emit_queue();
                self.want_play = playing;
                if self.start_item(id, position) {
                    if playing {
                        self.set_playing();
                    } else {
                        self.set_status(PlaybackStatus::Paused);
                    }
                }
            }
            Msg::SetDsp(mut dsp) => {
                if let Some(out) = &self.out {
                    dsp.configure(out.config.sample_rate, out.config.channels);
                }
                self.dsp = dsp;
                self.emit_output_info();
            }
            Msg::ReplaceStream(id, stream) => self.replace_stream(id, stream),
            Msg::SelectOutput(id) => self.select_output(id),
            Msg::Crossfade(duration) => {
                self.crossfade_duration = duration;
                if self.crossfade.is_some() && let Some(id) = self.queue.current_id() {
                    self.restart_current_exact(id);
                }
                self.fade_checked = None;
                self.emit_output_info();
            }
            Msg::SleepTimer(duration) => {
                self.sleep_deadline = duration.and_then(|duration| Instant::now().checked_add(duration));
            }
            Msg::State(reply) => {
                let _ = reply.send(self.snapshot());
            }
            Msg::Shutdown => self.running = false,
        }
    }

    fn command(&mut self, cmd: PlaybackCommand) {
        match cmd {
            PlaybackCommand::Load { items, start } => {
                // The device stays open: start_item flushes, and only reopens
                // it if the new track needs another format.
                self.reset_pipeline();
                self.queue.load(items, start);
                self.emit_queue();
                match self.queue.current_id() {
                    Some(id) => {
                        self.want_play = true;
                        if self.start_item(id, Duration::ZERO) {
                            self.set_playing();
                        }
                    }
                    None => self.stop(),
                }
            }
            PlaybackCommand::Enqueue { items } => {
                let was_empty = self.queue.is_empty();
                self.queue.enqueue(items);
                if was_empty && let Some(first) = self.queue.first_id() {
                    self.queue.set_current(first);
                }
                self.emit_queue();
                self.revalidate_prefetch();
            }
            PlaybackCommand::PlayNext { items } => {
                let was_empty = self.queue.is_empty();
                self.queue.play_next(items);
                if was_empty && let Some(first) = self.queue.first_id() {
                    self.queue.set_current(first);
                }
                self.emit_queue();
                self.revalidate_prefetch();
            }
            PlaybackCommand::Play => self.play(),
            PlaybackCommand::Pause => self.pause(),
            PlaybackCommand::TogglePause => match self.status {
                PlaybackStatus::Playing | PlaybackStatus::Buffering => self.pause(),
                _ => self.play(),
            },
            PlaybackCommand::Stop => self.stop(),
            PlaybackCommand::Seek { position } => self.seek(position),
            PlaybackCommand::SetVolume { volume } => {
                self.volume = if volume.is_finite() { volume.clamp(0.0, 1.0) } else { 1.0 };
                if let Some(out) = &self.out {
                    out.shared.volume.store(self.volume.to_bits(), Ordering::Relaxed);
                }
                self.emit(PlaybackEvent::VolumeChanged {
                    volume: self.volume,
                });
                self.emit_output_info();
            }
            PlaybackCommand::Next => {
                if let Some(cur) = self.queue.current_id() {
                    match self.queue.next_after(cur, false) {
                        Some(next) => self.select(next),
                        None => {
                            self.stop();
                            self.emit(PlaybackEvent::QueueFinished);
                        }
                    }
                }
            }
            PlaybackCommand::Previous => {
                if let Some(cur) = self.queue.current_id() {
                    let restart = self.position() > PREVIOUS_RESTART;
                    match self.queue.previous_before(cur) {
                        Some(prev) if !restart => self.select(prev),
                        _ => self.seek(Duration::ZERO),
                    }
                }
            }
            PlaybackCommand::JumpTo { id } => {
                if self.queue.get(id).is_some() {
                    let stopped = self.status == PlaybackStatus::Stopped;
                    if stopped {
                        self.want_play = true;
                    }
                    if self.start_item(id, Duration::ZERO) && stopped {
                        self.set_playing();
                    }
                }
            }
            PlaybackCommand::Remove { id } => {
                let was_current = self.queue.remove(id);
                self.emit_queue();
                if was_current {
                    match self.queue.current_id() {
                        Some(next) if self.status != PlaybackStatus::Stopped => {
                            self.start_item(next, Duration::ZERO);
                        }
                        Some(_) => self.emit_track_changed(),
                        None => self.stop(),
                    }
                } else {
                    self.revalidate_prefetch();
                }
            }
            PlaybackCommand::MoveBefore { id, before } => {
                if self.queue.move_before(id, before) {
                    self.emit_queue();
                    self.revalidate_prefetch();
                }
            }
            PlaybackCommand::Clear => {
                self.stop();
                self.queue.clear();
                self.emit_queue();
                self.emit(PlaybackEvent::TrackChanged {
                    item: None,
                    duration: None,
                });
            }
            PlaybackCommand::SetRepeat { mode } => {
                self.queue.set_repeat(mode);
                self.emit(PlaybackEvent::RepeatChanged { mode });
                self.revalidate_prefetch();
            }
            PlaybackCommand::SetShuffle { enabled } => {
                self.queue.set_shuffle(enabled);
                self.emit(PlaybackEvent::ShuffleChanged { enabled });
                self.emit_queue();
                self.revalidate_prefetch();
            }
        }
    }

    fn set_status(&mut self, status: PlaybackStatus) {
        if let Some(out) = &self.out {
            out.shared
                .paused
                .store(status != PlaybackStatus::Playing, Ordering::Release);
        }
        if status != self.status {
            self.status = status;
            self.emit(PlaybackEvent::StatusChanged { status });
        }
    }

    /// Skips to `id`: starts it if a track is active, otherwise only moves
    /// the cursor (the device is not opened while stopped).
    fn select(&mut self, id: QueueItemId) {
        if self.status == PlaybackStatus::Stopped {
            self.queue.set_current(id);
            self.emit_track_changed();
            self.emit_queue();
        } else {
            self.start_item(id, Duration::ZERO);
        }
    }

    fn play(&mut self) {
        if self.status != PlaybackStatus::Stopped
            && let Some(stall) = self.stall.take_if(|s| s.kind == StallKind::Network)
        {
            // Retry the network from where it stopped.
            self.want_play = true;
            self.resume_stall(stall);
            self.refresh_status();
            return;
        }
        match self.status {
            PlaybackStatus::Playing => {}
            PlaybackStatus::Paused | PlaybackStatus::Buffering
                if self.out.is_some() || self.opening.is_some() || self.stall.is_some() =>
            {
                self.set_playing();
            }
            _ => {
                let id = self.queue.current_id().or_else(|| self.queue.first_id());
                self.want_play = true;
                if let Some(id) = id
                    && self.start_item(id, Duration::ZERO)
                {
                    self.set_playing();
                }
            }
        }
    }

    fn pause(&mut self) {
        self.want_play = false;
        if matches!(self.status, PlaybackStatus::Playing | PlaybackStatus::Buffering) {
            self.set_status(PlaybackStatus::Paused);
        }
    }

    /// The user wants sound: Playing, or Buffering while the network catches up.
    fn set_playing(&mut self) {
        self.want_play = true;
        let status = if self.waiting() {
            PlaybackStatus::Buffering
        } else {
            PlaybackStatus::Playing
        };
        self.set_status(status);
    }

    /// Recomputes Playing/Buffering/Paused after a streaming state change.
    fn refresh_status(&mut self) {
        if self.status == PlaybackStatus::Stopped {
            return;
        }
        let blocked = self.underrun || self.stall_blocks_current();
        if self.want_play
            && blocked
            && self.stall.as_ref().is_some_and(|s| s.kind == StallKind::Network)
        {
            // Out of audio and no network: pause; play() retries.
            self.want_play = false;
        }
        if self.want_play {
            self.set_playing();
        } else {
            self.set_status(PlaybackStatus::Paused);
        }
    }

    /// Waiting for the network before sound can (continue to) play.
    fn waiting(&self) -> bool {
        self.opening.is_some() || self.underrun || self.stall_blocks_current()
    }

    /// The stalled item is the one that should be heard now.
    fn stall_blocks_current(&self) -> bool {
        self.segments.is_empty()
            && self
                .stall
                .as_ref()
                .is_some_and(|s| Some(s.id) == self.queue.current_id())
    }

    fn stop(&mut self) {
        self.want_play = false;
        self.stop_output();
        self.set_status(PlaybackStatus::Stopped);
        self.emit(PlaybackEvent::Position {
            position: Duration::ZERO,
            duration: None,
        });
    }

    fn select_output(&mut self, device_id: Option<String>) {
        let position = self.position();
        let active = self.status != PlaybackStatus::Stopped;
        self.stop_output();
        let replacement: Box<dyn AudioSink> = match device_id {
            Some(id) => Box::new(CpalSink::with_device(id)),
            None => Box::new(CpalSink::new()),
        };
        let previous = std::mem::replace(&mut self.sink, replacement);
        if active && let Some(id) = self.queue.current_id() {
            let want_play = self.want_play;
            if !self.start_item(id, position) {
                self.stop_output();
                self.sink = previous;
                self.want_play = want_play;
                if self.start_item(id, position) {
                    if want_play {
                        self.set_playing();
                    } else {
                        self.set_status(PlaybackStatus::Paused);
                    }
                }
            } else {
                self.refresh_status();
            }
        }
    }

    /// Drops decoding state and releases the device.
    fn stop_output(&mut self) {
        self.reset_pipeline();
        self.preopen = None;
        if self.out.take().is_some() {
            self.sink.close();
        }
    }

    fn reset_pipeline(&mut self) {
        self.crossfade = None;
        self.fade_checked = None;
        self.segments.clear();
        self.dec = None;
        self.pending_reopen = None;
        self.tail = None;
        self.opening = None;
        self.next_opening = None;
        self.stall = None;
        self.underrun = false;
    }

    /// Discards everything buffered (ring included) without closing.
    fn flush(&mut self) {
        if let Some(out) = &mut self.out {
            out.flush_gen += 1;
            out.shared.flush_req.store(out.flush_gen, Ordering::Release);
        }
        self.reset_pipeline();
        self.dsp.reset();
    }

    fn seek(&mut self, position: Duration) {
        if let Some(cur) = self.queue.current_id()
            && self.status != PlaybackStatus::Stopped
        {
            self.start_item(cur, position);
        }
    }

    /// Flushes and starts decoding `id` at `position`. Unplayable items are
    /// reported and skipped. Remote items start opening in the background
    /// (status `Buffering`). Returns false when nothing could be started.
    fn start_item(&mut self, id: QueueItemId, position: Duration) -> bool {
        self.flush();
        let mut candidate = Some(id);
        let mut attempts = 0;
        while let Some(id) = candidate {
            attempts += 1;
            if attempts > self.queue.len().max(1) {
                break;
            }
            let feed = match self.open_item(id, position) {
                Ok(Opened::Ready(feed)) => feed,
                Ok(Opened::Waiting(feed)) => {
                    self.opening = Some(Opening { id, position, feed });
                    self.queue.set_current(id);
                    self.emit_track_changed();
                    self.emit_queue();
                    self.emit_position();
                    self.refresh_status();
                    return true;
                }
                Err(e) => {
                    self.emit_error(Some(id), &e);
                    candidate = self.queue.next_after(id, false);
                    continue;
                }
            };
            let Feed::Local(mut decoder) = feed else {
                candidate = self.queue.next_after(id, false);
                continue;
            };
            let mut offset = Duration::ZERO;
            if !position.is_zero() {
                match decoder.seek(position) {
                    Ok(actual) => offset = actual,
                    Err(e) => self.emit_error(Some(id), &e),
                }
            }
            if let Err(e) = self.ensure_output(decoder.spec()) {
                self.emit_error(Some(id), &e);
                self.stop_output();
                self.set_status(PlaybackStatus::Stopped);
                return false;
            }
            if let Err(e) = self.begin_segment(id, Feed::Local(decoder), offset) {
                self.emit_error(Some(id), &e);
                candidate = self.queue.next_after(id, false);
                continue;
            }
            self.queue.set_current(id);
            self.emit_track_changed();
            self.emit_queue();
            self.emit_position();
            return true;
        }
        self.stop();
        self.emit(PlaybackEvent::QueueFinished);
        false
    }

    /// Opens a queue item: local files (and complete cached streams) at
    /// once, remote streams on a worker thread, already seeked to `position`.
    fn open_item(&mut self, id: QueueItemId, position: Duration) -> Result<Opened> {
        let item = self
            .queue
            .get(id)
            .ok_or_else(|| Error::NotFound(format!("queue item {id}")))?;
        if let StreamSource::File { .. } = &item.stream {
            let mut decoder = TrackDecoder::open(&item.stream)?;
            decoder.reconcile_duration(item.track.duration);
            return Ok(Opened::Ready(Feed::Local(decoder)));
        }
        if let Some(cache) = &self.net.cache
            && let Some(path) = cache.cached(&item.track)
        {
            match TrackDecoder::open_file(&path) {
                Ok(mut decoder) => {
                    log::debug!("playing {id} from the stream cache");
                    decoder.reconcile_duration(item.track.duration);
                    return Ok(Opened::Ready(Feed::Local(decoder)));
                }
                Err(e) => {
                    log::warn!("unreadable cache entry {}: {e}", path.display());
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        if position.is_zero()
            && let Some((_, feed)) = self.preopen.take_if(|(pid, _)| *pid == id)
        {
            return Ok(Opened::Waiting(feed));
        }
        let cache_key = self.net.cache.as_ref().and_then(|_| StreamCache::key_for(&item.track));
        let StreamSource::Url { url, headers, format } = &item.stream else {
            let resolver = self.resolver.clone().ok_or_else(|| Error::Unsupported("no stream resolver".into()))?;
            let track = item.track.clone();
            let feed = RemoteFeed::spawn_with(&self.net, move |link| {
                let source = resolver.open(&track, || link.is_cancelled()).map_err(|error| match error {
                    Error::Network(message) => FeedFailure::Network(message),
                    other => FeedFailure::Fatal(other),
                })?;
                let StreamSource::Url { url, headers, format } = source else {
                    return Err(FeedFailure::Fatal(Error::Unsupported("resolver did not return a remote stream".into())));
                };
                Ok(RemoteSpec { url, headers, format, position, duration_hint: track.duration, cache_key })
            });
            return Ok(Opened::Waiting(feed));
        };
        let spec = RemoteSpec {
            url: url.clone(),
            headers: headers.clone(),
            format: format.clone(),
            position,
            duration_hint: item.track.duration,
            cache_key,
        };
        Ok(Opened::Waiting(RemoteFeed::spawn(&self.net, spec)))
    }

    /// Opens (or reopens) the output if it cannot carry `spec` as is.
    fn ensure_output(&mut self, spec: SourceSpec) -> Result<()> {
        if self.out.as_ref().is_some_and(|o| o.accepts(spec)) {
            return Ok(());
        }
        if self.out.take().is_some() {
            self.sink.close();
        }
        let request = OutputRequest {
            sample_rate: spec.sample_rate,
            channels: spec.channels,
            bits_per_sample: spec.bits_per_sample,
        };
        let volume = self.volume;
        let paused = self.status != PlaybackStatus::Playing;
        let buffer = self.config.buffer;
        let mut built: Option<(HeapProd<f32>, Arc<Shared>)> = None;
        let config = self
            .sink
            .open(request, &mut |cfg: &OutputConfig| {
                let frames = (buffer.as_secs_f64() * f64::from(cfg.sample_rate)) as usize;
                let channels = usize::from(cfg.channels.max(1));
                let ring = HeapRb::<f32>::new(frames.max(8192) * channels);
                let (prod, cons) = ring.split();
                let shared = Arc::new(Shared::new(volume));
                shared.paused.store(paused, Ordering::Release);
                built = Some((prod, Arc::clone(&shared)));
                Renderer::new(cons, shared, cfg.channels)
            })
            .map_err(Error::Output)?;
        let (prod, shared) =
            built.ok_or_else(|| Error::Output("sink did not create a renderer".into()))?;
        self.dsp.configure(config.sample_rate, config.channels);
        self.out = Some(Output {
            config,
            request,
            shared,
            prod,
            written: 0,
            flush_gen: 0,
        });
        Ok(())
    }

    /// Starts decoding `id` (an opened feed) into the current output,
    /// appending a segment.
    fn begin_segment(&mut self, id: QueueItemId, feed: Feed, offset: Duration) -> Result<()> {
        let out = self
            .out
            .as_ref()
            .ok_or_else(|| Error::Output("no output".into()))?;
        let (spec, duration, format) = feed
            .info()
            .ok_or_else(|| Error::Decode("stream not open".into()))?;
        let segment = Segment {
            id,
            start: out.written,
            end: None,
            offset,
            rate: out.config.sample_rate,
            duration,
            format,
            source_channels: spec.channels,
            resampled_from: (spec.sample_rate != out.config.sample_rate).then_some(spec.sample_rate),
        };
        self.dec = Some(Decoding::new(id, feed, spec, offset, &out.config)?);
        self.segments.push_back(segment);
        Ok(())
    }

    /// Only overlap local, exact-rate PCM-compatible tracks. If the next
    /// file cannot be prepared, the unmodified gapless path takes over.
    fn prepare_crossfade(&mut self) {
        if self.crossfade_duration.is_zero() || self.crossfade.is_some() || !self.dsp.is_bypassed() {
            return;
        }
        let (Some(dec), Some(out), Some(segment)) = (&self.dec, &self.out, self.segments.back()) else {
            return;
        };
        let supported = |item: Option<&QueueItem>| {
            item.and_then(|item| match &item.stream {
                StreamSource::File { path } => path.extension().and_then(|ext| ext.to_str()),
                _ => None,
            }).is_some_and(|ext| ext.eq_ignore_ascii_case("wav") || ext.eq_ignore_ascii_case("flac"))
        };
        let Some(next) = self.queue.next_after(dec.id, true).filter(|id| *id != dec.id) else { return };
        if self.fade_checked == Some((dec.id, next))
            || !supported(self.queue.get(dec.id)) || !supported(self.queue.get(next))
            || dec.spec.sample_rate != out.config.sample_rate || dec.spec.channels != out.config.channels
            || out.config.channels > 2 || out.config.sample_rate > 96_000
        {
            return;
        }
        let Some(remaining) = segment.duration.map(|duration| duration.saturating_sub(segment.offset)) else {
            return;
        };
        let rate = u64::from(out.config.sample_rate);
        let remaining_frames = (remaining.as_secs_f64() * rate as f64).round() as u64;
        let wanted = ((self.crossfade_duration.as_secs_f64() * rate as f64) as u64).min(remaining_frames / 2);
        let end = segment.start.saturating_add(remaining_frames);
        let within_prebuffer = end.saturating_sub(out.written) <= wanted.saturating_add(rate);
        if wanted == 0 || !within_prebuffer || end <= out.written {
            return;
        }
        let from = dec.id;
        let config = out.config.clone();
        let written = out.written;
        self.fade_checked = Some((from, next));

        let Some(StreamSource::File { path }) = self.queue.get(next).map(|item| &item.stream) else { return };
        let Ok(decoder) = TrackDecoder::open_file(path) else { return };
        let spec = decoder.spec();
        if spec.sample_rate != config.sample_rate || spec.channels != config.channels || !depth_ok(config.sample_format.bits(), spec.bits_per_sample) {
            return;
        }
        let Ok(mut incoming) = Decoding::new(next, Feed::Local(decoder), spec, Duration::ZERO, &config) else { return };
        let channels = incoming.out_channels;
        let wanted = wanted.min(end - written) as usize;
        let mut bypass = Bypass;
        for _ in 0..1024 {
            if incoming.pcm.len() / channels >= wanted || incoming.eof {
                break;
            }
            match incoming.fill(&mut bypass) {
                Ok(true) => {}
                Ok(false) | Err(_) => return,
            }
        }
        let available = incoming.pcm.len() / channels;
        if available < wanted && !incoming.eof {
            return;
        }
        let frames = if incoming.eof { wanted.min(available / 2) } else { wanted };
        if frames == 0 {
            return;
        }
        self.crossfade = Some(Crossfade {
            from, incoming, at: end - frames as u64, frames, mixed: 0, scratch: Vec::new(),
        });
    }

    /// Feeds the ring. Returns true if any progress was made.
    fn pump(&mut self) -> bool {
        let mut worked = self.poll_opening();
        worked |= self.poll_next_opening();
        self.prepare_crossfade();
        let Some(out) = self.out.as_mut() else {
            return worked;
        };
        if out.flush_pending() {
            return worked;
        }
        let mut failure = None;
        for _ in 0..PUMP_BUDGET {
            let Some(dec) = self.dec.as_mut() else {
                break;
            };
            if dec.has_pending() {
                let pushed = if let Some(fade) = self.crossfade.as_mut().filter(|fade| fade.from == dec.id) {
                    fade.push(dec, out)
                } else {
                    dec.push(&mut out.prod, usize::MAX)
                };
                out.written += pushed;
                if pushed == 0 {
                    break; // ring full
                }
                worked = true;
                continue;
            }
            if dec.eof {
                let finished = dec.id;
                if let Some(seg) = self.segments.back_mut()
                    && seg.end.is_none()
                {
                    seg.end = Some(out.written);
                }
                self.dec = None;
                if let Some(fade) = self.crossfade.take()
                    && fade.from == finished && fade.mixed > 0
                    && let Some((spec, duration, format)) = fade.incoming.feed.info()
                {
                    self.segments.push_back(Segment {
                        id: fade.incoming.id, start: out.written, end: None,
                        offset: frames_to_duration(fade.mixed as u64, out.config.sample_rate),
                        rate: out.config.sample_rate, duration, format,
                        source_channels: spec.channels, resampled_from: None,
                    });
                    self.dec = Some(fade.incoming);
                    return true;
                }
                self.prefetch_after(finished);
                return true;
            }
            match dec.fill(self.dsp.as_mut()) {
                Ok(true) => worked = true,
                Ok(false) => break, // remote source has nothing yet
                Err(f) => {
                    let position = dec.resume_position();
                    if matches!(f, FeedFailure::Fatal(_)) {
                        // Broken stream: play what was decoded, then move on.
                        dec.eof = true;
                    } else {
                        dec.feed = Feed::Dead;
                    }
                    failure = Some((dec.id, position, f));
                    worked = true;
                    break;
                }
            }
        }
        if let Some((id, position, f)) = failure {
            match f {
                FeedFailure::Fatal(e) => self.emit_error(Some(id), &e),
                other => self.stall_item(id, position, other),
            }
        }

        if self.dec.is_none()
            && self.pending_reopen.is_none()
            && self.next_opening.is_none()
            && let Some(tail) = self.tail
            && self.queue.next_after(tail, true).is_some()
        {
            self.tail = None;
            self.prefetch_after(tail);
            worked = true;
        }
        worked
    }

    /// Finishes starting the current item once its remote stream is open.
    fn poll_opening(&mut self) -> bool {
        let Some(opening) = self.opening.as_mut() else {
            return false;
        };
        match opening.feed.poll_open() {
            OpenPoll::Pending => false,
            OpenPoll::Ready => {
                let Some(Opening { id, position, feed }) = self.opening.take() else {
                    return false;
                };
                self.failed_opens = 0;
                let feed = Feed::Remote(feed);
                let Some((spec, duration, _)) = feed.info() else {
                    return false;
                };
                if let Err(e) = self.ensure_output(spec) {
                    self.emit_error(Some(id), &e);
                    self.stop();
                    return true;
                }
                let offset = duration.map_or(position, |d| position.min(d));
                if let Err(e) = self.begin_segment(id, feed, offset) {
                    self.emit_error(Some(id), &e);
                    self.skip_after(id);
                    return true;
                }
                // Let the ring fill before sound starts.
                self.underrun = true;
                self.emit_output_info();
                self.emit_position();
                self.refresh_status();
                true
            }
            OpenPoll::Failed(f) => {
                let Some(Opening { id, position, .. }) = self.opening.take() else {
                    return false;
                };
                match f {
                    FeedFailure::Fatal(e) => {
                        self.emit_error(Some(id), &e);
                        self.skip_after(id);
                    }
                    other => self.stall_item(id, position, other),
                }
                self.refresh_status();
                true
            }
        }
    }

    /// Attaches the next item once its remote stream is open.
    fn poll_next_opening(&mut self) -> bool {
        let Some((_, feed)) = self.next_opening.as_mut() else {
            return false;
        };
        match feed.poll_open() {
            OpenPoll::Pending => false,
            OpenPoll::Ready => {
                if let Some((id, feed)) = self.next_opening.take() {
                    self.failed_opens = 0;
                    self.attach_next(id, Feed::Remote(feed));
                }
                true
            }
            OpenPoll::Failed(f) => {
                if let Some((id, _)) = self.next_opening.take() {
                    match f {
                        FeedFailure::Fatal(e) => {
                            self.emit_error(Some(id), &e);
                            self.failed_opens += 1;
                            if self.failed_opens > self.queue.len() {
                                self.tail = None;
                            } else {
                                self.prefetch_after(id);
                            }
                        }
                        other => self.stall_item(id, Duration::ZERO, other),
                    }
                    self.refresh_status();
                }
                true
            }
        }
    }

    /// Appends an opened next item after the current one: gapless when the
    /// output accepts its format, otherwise after the ring drains.
    fn attach_next(&mut self, id: QueueItemId, feed: Feed) {
        let Some((spec, _, _)) = feed.info() else {
            return;
        };
        if self.segments.is_empty() {
            // The previous track already finished playing: start now.
            let res = self
                .ensure_output(spec)
                .and_then(|()| self.begin_segment(id, feed, Duration::ZERO));
            match res {
                Ok(()) => {
                    if self.queue.current_id() != Some(id) {
                        self.queue.set_current(id);
                        self.emit_track_changed();
                        self.emit_queue();
                    }
                    self.underrun = true;
                    self.emit_output_info();
                    self.emit_position();
                }
                Err(e) => {
                    self.emit_error(Some(id), &e);
                    self.skip_after(id);
                }
            }
        } else if self.out.as_ref().is_some_and(|o| o.accepts(spec)) {
            if let Err(e) = self.begin_segment(id, feed, Duration::ZERO) {
                self.emit_error(Some(id), &e);
                self.prefetch_after(id);
            }
        } else {
            self.pending_reopen = Some((id, feed));
        }
        self.refresh_status();
    }

    /// `id` cannot be played: start the item after it, or end the queue.
    fn skip_after(&mut self, id: QueueItemId) {
        self.failed_opens += 1;
        let next = self.queue.next_after(id, false);
        match next {
            Some(next) if self.failed_opens <= self.queue.len() => {
                self.start_item(next, Duration::ZERO);
                if self.want_play {
                    self.set_playing();
                }
            }
            _ => {
                self.failed_opens = 0;
                self.finish_queue();
            }
        }
    }

    /// Records a stream stall (expired URL / network down) and notifies.
    fn stall_item(&mut self, id: QueueItemId, position: Duration, failure: FeedFailure) {
        let kind = match failure {
            FeedFailure::Expired => StallKind::Expired,
            FeedFailure::Network(_) => StallKind::Network,
            FeedFailure::Fatal(e) => {
                self.emit_error(Some(id), &e);
                self.give_up(id);
                return;
            }
        };
        if kind == StallKind::Expired {
            if self.last_expired == Some((id, position)) {
                // The refreshed URL failed at the same point: give up.
                self.emit_error(
                    Some(id),
                    &Error::Network("stream URL still refused after refreshing".into()),
                );
                self.last_expired = None;
                self.give_up(id);
                return;
            }
            self.last_expired = Some((id, position));
            log::info!("stream of {id} expired at {position:?}; waiting for a new URL");
            self.emit_stream(StreamEvent::Expired { id, position });
        } else {
            let message = failure.to_error().to_string();
            self.emit_error(Some(id), &failure.to_error());
            self.emit_stream(StreamEvent::NetworkFailed {
                id,
                position,
                message,
            });
        }
        self.stall = Some(Stall {
            id,
            position,
            kind,
            since: Instant::now(),
        });
    }

    /// Stops waiting for a stalled item and moves past it.
    fn give_up(&mut self, id: QueueItemId) {
        if self.stall.as_ref().is_some_and(|s| s.id == id) {
            self.stall = None;
        }
        if let Some(dec) = self.dec.as_mut()
            && dec.id == id
        {
            // Play what was decoded; the normal EOF path continues the queue.
            dec.eof = true;
        } else if self.segments.is_empty() {
            self.skip_after(id);
        } else {
            self.prefetch_after(id);
        }
        self.refresh_status();
    }

    /// Resumes a stalled item at its stall position.
    fn resume_stall(&mut self, stall: Stall) {
        let Stall { id, position, .. } = stall;
        if self.dec.as_ref().is_some_and(|d| d.id == id) {
            // Mid-track: reconnect behind the audio still buffered (gapless).
            match self.open_item(id, position) {
                Ok(Opened::Waiting(feed)) => {
                    if let Some(dec) = self.dec.as_mut() {
                        dec.feed = Feed::Remote(feed);
                    }
                }
                Ok(Opened::Ready(Feed::Local(mut decoder))) => {
                    let ok = decoder.seek(position).is_ok() && decoder.spec() == self.dec.as_ref().map_or(decoder.spec(), |d| d.spec);
                    if let Some(dec) = self.dec.as_mut() {
                        if ok {
                            dec.feed = Feed::Local(decoder);
                        } else {
                            dec.eof = true;
                        }
                    }
                }
                Ok(Opened::Ready(_)) => {}
                Err(e) => {
                    self.emit_error(Some(id), &e);
                    if let Some(dec) = self.dec.as_mut() {
                        dec.eof = true;
                    }
                }
            }
        } else if self.segments.is_empty() {
            self.start_item(id, position);
        } else {
            // The next track: open it behind the current one.
            match self.open_item(id, Duration::ZERO) {
                Ok(Opened::Waiting(feed)) => self.next_opening = Some((id, feed)),
                Ok(Opened::Ready(feed)) => self.attach_next(id, feed),
                Err(e) => {
                    self.emit_error(Some(id), &e);
                    self.prefetch_after(id);
                }
            }
        }
    }

    /// `replace_stream`: new URL for an item; resumes it if it was stalled.
    fn replace_stream(&mut self, id: QueueItemId, stream: StreamSource) {
        if !self.queue.set_stream(id, stream) {
            return;
        }
        if self.preopen.as_ref().is_some_and(|(pid, _)| *pid == id) {
            self.preopen = None;
        }
        if self.next_opening.as_ref().is_some_and(|(nid, _)| *nid == id) {
            self.next_opening = None;
            self.stall = Some(Stall {
                id,
                position: Duration::ZERO,
                kind: StallKind::Expired,
                since: Instant::now(),
            });
        }
        if let Some(stall) = self.stall.take_if(|s| s.id == id) {
            self.resume_stall(stall);
            self.refresh_status();
        }
    }

    /// Opens the item following `prev` for gapless continuation.
    fn prefetch_after(&mut self, prev: QueueItemId) {
        let mut candidate = self.queue.next_after(prev, true);
        let mut attempts = 0;
        while let Some(id) = candidate {
            attempts += 1;
            if attempts > self.queue.len() {
                break;
            }
            match self.open_item(id, Duration::ZERO) {
                Ok(Opened::Ready(feed)) => {
                    let gapless = feed
                        .info()
                        .is_some_and(|(spec, _, _)| self.out.as_ref().is_some_and(|o| o.accepts(spec)));
                    if gapless {
                        if let Err(e) = self.begin_segment(id, feed, Duration::ZERO) {
                            self.emit_error(Some(id), &e);
                            candidate = self.queue.next_after(id, false);
                            continue;
                        }
                    } else {
                        self.pending_reopen = Some((id, feed));
                    }
                    return;
                }
                Ok(Opened::Waiting(feed)) => {
                    self.next_opening = Some((id, feed));
                    return;
                }
                Err(e) => {
                    self.emit_error(Some(id), &e);
                    candidate = self.queue.next_after(id, false);
                }
            }
        }
        self.tail = if attempts > self.queue.len() { None } else { Some(prev) };
    }

    /// Re-decodes from the current position if what was pre-decoded after
    /// the current track no longer matches the queue.
    fn revalidate_prefetch(&mut self) {
        let Some(cur) = self.queue.current_id() else {
            return;
        };
        if self.status == PlaybackStatus::Stopped || self.segments.is_empty() {
            return;
        }
        let next = self.queue.next_after(cur, true);
        if self.crossfade.as_ref().is_some_and(|fade| {
            self.queue.next_after(fade.from, true) != Some(fade.incoming.id)
        }) {
            self.restart_current_exact(cur);
            return;
        }
        if self.preopen.as_ref().is_some_and(|(id, _)| Some(*id) != next) {
            self.preopen = None;
        }
        if self.stall.as_ref().is_some_and(|s| s.id != cur && Some(s.id) != next) {
            // A stalled next item that is no longer next.
            self.stall = None;
        }
        let prefetched = self
            .segments
            .get(1)
            .map(|s| s.id)
            .or_else(|| self.pending_reopen.as_ref().map(|(id, _)| *id))
            .or_else(|| self.next_opening.as_ref().map(|(id, _)| *id));
        if let Some(p) = prefetched
            && Some(p) != next
        {
            self.restart_current_exact(cur);
        }
    }

    /// Discards everything decoded ahead and resumes `cur` exactly at the
    /// frame the listener is hearing, so nothing is skipped or repeated.
    fn restart_current_exact(&mut self, cur: QueueItemId) {
        let Some(out) = self.out.as_mut() else {
            return;
        };
        out.flush_gen += 1;
        out.shared.flush_req.store(out.flush_gen, Ordering::Release);
        let deadline = Instant::now() + FLUSH_WAIT;
        while out.flush_pending() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        let heard = if out.flush_pending() {
            out.played()
        } else {
            out.shared.flushed_at.load(Ordering::Acquire)
        };
        // Retire the segments that were fully heard before the flush; the
        // audible one may already be the next (pre-decoded) track.
        let mut last_ended = None;
        while let Some(front) = self.segments.front()
            && front.end.is_some_and(|end| heard >= end)
        {
            let ended = front.id;
            self.segments.pop_front();
            self.emit(PlaybackEvent::TrackEnded { id: ended });
            last_ended = Some(ended);
        }
        match self.segments.front() {
            Some(front) => {
                let id = front.id;
                let position =
                    front.offset + frames_to_duration(heard.saturating_sub(front.start), front.rate);
                self.start_item(id, position);
            }
            None => {
                let after = last_ended.unwrap_or(cur);
                match self.queue.next_after(after, true) {
                    Some(next) => {
                        self.start_item(next, Duration::ZERO);
                    }
                    None => self.finish_queue(),
                }
            }
        }
    }

    /// Network buffering, early pre-open of the next remote track and the
    /// wait for refreshed URLs.
    fn stream_housekeeping(&mut self) {
        // Expired URL never refreshed: skip the item.
        if let Some(stall) = &self.stall
            && stall.kind == StallKind::Expired
            && stall.since.elapsed() > self.net.config.expired_wait
        {
            let id = stall.id;
            self.stall = None;
            self.emit_error(
                Some(id),
                &Error::Network("stream URL expired and was not refreshed".into()),
            );
            self.give_up(id);
        }

        // Underrun handling for remote sources.
        if let Some(out) = self.out.as_ref()
            && !out.flush_pending()
        {
            let channels = usize::from(out.config.channels.max(1));
            let capacity = out.prod.capacity().get() / channels;
            let occupied = out.prod.occupied_len() / channels;
            let starving = self.dec.as_ref().is_some_and(|d| d.starved && !d.eof && !d.has_pending())
                || (self.dec.is_none() && (self.next_opening.is_some() || self.stall.is_some()));
            let more_coming = self.dec.as_ref().is_some_and(|d| !d.eof) || self.next_opening.is_some();
            if !self.underrun {
                if self.status == PlaybackStatus::Playing
                    && starving
                    && (occupied as f64) < capacity as f64 * LOW_FILL
                {
                    log::debug!("network underrun: buffering");
                    self.underrun = true;
                }
            } else if (occupied as f64) >= capacity as f64 * RESUME_FILL
                || (!more_coming && self.stall.is_none())
                || (!starving && self.dec.as_ref().is_some_and(|d| d.eof))
            {
                self.underrun = false;
            }
        }

        // Open the next remote track before the current one ends.
        if let Some(dec) = &self.dec {
            let next = self.queue.next_after(dec.id, true).filter(|&n| n != dec.id);
            if self.preopen.as_ref().is_some_and(|(id, _)| Some(*id) != next) {
                self.preopen = None;
            }
            let remaining = self
                .segments
                .back()
                .and_then(|s| s.duration)
                .map(|d| d.saturating_sub(dec.resume_position()));
            if self.want_play
                && self.preopen.is_none()
                && self.next_opening.is_none()
                && let Some(next) = next
                && self.preopen_cached != Some(next)
                && remaining.is_some_and(|r| r <= self.net.config.prebuffer)
                && self.queue.get(next).is_some_and(|item| !matches!(&item.stream, StreamSource::File { .. }))
            {
                match self.open_item(next, Duration::ZERO) {
                    Ok(Opened::Waiting(feed)) => self.preopen = Some((next, feed)),
                    Ok(Opened::Ready(_)) => self.preopen_cached = Some(next),
                    Err(error) => {
                        self.preopen_cached = Some(next);
                        self.emit_error(Some(next), &error);
                    }
                }
            }
        }
        self.refresh_status();
    }

    /// Tracks segment boundaries and emits position/track events.
    fn update(&mut self) {
        if self.sleep_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            self.sleep_deadline = None;
            self.pause();
            self.emit(PlaybackEvent::SleepTimerElapsed);
        }
        if self.out.is_some() && self.sink.failed() {
            let position = self.position();
            self.emit(PlaybackEvent::Error {
                id: self.queue.current_id(),
                message: "audio device lost; reopening".into(),
            });
            self.stop_output();
            if let Some(cur) = self.queue.current_id() {
                self.start_item(cur, position);
            }
            return;
        }
        self.stream_housekeeping();

        loop {
            let Some(out) = self.out.as_ref() else {
                return;
            };
            if out.flush_pending() {
                break;
            }
            let played = out.played();
            let Some(front) = self.segments.front() else {
                break;
            };
            let Some(end) = front.end else {
                break;
            };
            if played < end {
                break;
            }
            let ended = front.id;
            self.segments.pop_front();
            self.emit(PlaybackEvent::TrackEnded { id: ended });

            if let Some(next) = self.segments.front() {
                let id = next.id;
                self.queue.set_current(id);
                self.emit_track_changed();
                self.emit_queue();
            } else if let Some((id, feed)) = self.pending_reopen.take() {
                let res = match feed.info() {
                    Some((spec, _, _)) => self
                        .ensure_output(spec)
                        .and_then(|()| self.begin_segment(id, feed, Duration::ZERO)),
                    None => Err(Error::Decode("stream not open".into())),
                };
                match res {
                    Ok(()) => {
                        self.queue.set_current(id);
                        self.emit_track_changed();
                        self.emit_queue();
                    }
                    Err(e) => {
                        self.emit_error(Some(id), &e);
                        if let Some(next) = self.queue.next_after(id, false) {
                            self.start_item(next, Duration::ZERO);
                        } else {
                            self.finish_queue();
                        }
                        return;
                    }
                }
            } else if let Some(waiting) = self
                .next_opening
                .as_ref()
                .map(|(id, _)| *id)
                .or_else(|| self.stall.as_ref().map(|s| s.id))
            {
                // The next track is still on its way over the network.
                if self.queue.current_id() != Some(waiting) {
                    self.queue.set_current(waiting);
                    self.emit_track_changed();
                    self.emit_queue();
                }
                self.refresh_status();
                break;
            } else if self.dec.is_none() {
                self.finish_queue();
                return;
            }
        }

        if self.status == PlaybackStatus::Playing
            && self.last_position.elapsed() >= self.config.position_interval
        {
            self.emit_position();
        }
    }

    fn finish_queue(&mut self) {
        self.stop_output();
        self.set_status(PlaybackStatus::Stopped);
        self.emit(PlaybackEvent::Position {
            position: Duration::ZERO,
            duration: None,
        });
        self.emit(PlaybackEvent::QueueFinished);
    }

    fn position(&self) -> Duration {
        if self.segments.is_empty() {
            if let Some(op) = &self.opening {
                return op.position;
            }
            if let Some(stall) = &self.stall
                && Some(stall.id) == self.queue.current_id()
            {
                return stall.position;
            }
        }
        let (Some(out), Some(seg)) = (self.out.as_ref(), self.segments.front()) else {
            return Duration::ZERO;
        };
        let mut played = out.played();
        if let Some(end) = seg.end {
            played = played.min(end);
        }
        let frames = played.saturating_sub(seg.start);
        seg.offset + frames_to_duration(frames, seg.rate)
    }

    /// Duration of the current track: the decoder's, else the metadata's.
    fn current_duration(&self) -> Option<Duration> {
        self.segments
            .front()
            .and_then(|s| s.duration)
            .or_else(|| self.queue.current().and_then(|i| i.track.duration))
    }

    fn emit_position(&mut self) {
        self.last_position = Instant::now();
        let duration = self.current_duration();
        self.emit(PlaybackEvent::Position {
            position: self.position(),
            duration,
        });
    }

    fn emit_track_changed(&mut self) {
        let item = self.queue.current().cloned().map(Box::new);
        let duration = self.current_duration();
        self.emit(PlaybackEvent::TrackChanged { item, duration });
        self.emit_output_info();
    }

    fn emit_queue(&self) {
        self.emit(PlaybackEvent::QueueChanged {
            items: self.queue.ordered_items(),
            current: self.queue.current_position(),
        });
    }

    fn emit_output_info(&self) {
        if let Some(info) = self.output_info() {
            self.emit(PlaybackEvent::OutputChanged { info });
        }
    }

    fn emit_stream(&self, event: StreamEvent) {
        match self.stream_events.try_send(event) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => log::trace!("stream event channel full, event dropped"),
        }
    }

    fn emit_error(&self, id: Option<QueueItemId>, error: &Error) {
        log::warn!("playback error ({id:?}): {error}");
        self.emit(PlaybackEvent::Error {
            id,
            message: error.to_string(),
        });
    }

    fn output_info(&self) -> Option<OutputInfo> {
        let out = self.out.as_ref()?;
        let seg = self.segments.front();
        let bit_perfect = seg.is_some_and(|s| {
            s.resampled_from.is_none()
                && s.source_channels == out.config.channels
                && depth_ok(
                    out.config.sample_format.bits(),
                    s.format.bit_depth.map(u32::from),
                )
        }) && self.dsp.is_bypassed()
            && self.crossfade_duration.is_zero()
            && self.volume == 1.0;
        Some(OutputInfo {
            device: out.config.device.clone(),
            sample_rate: out.config.sample_rate,
            channels: out.config.channels,
            source: seg.map(|s| s.format.clone()),
            bit_perfect,
            resampled_from: seg.and_then(|s| s.resampled_from),
        })
    }

    fn snapshot(&self) -> PlaybackState {
        PlaybackState {
            status: self.status,
            queue: self.queue.ordered_items(),
            current: self.queue.current_position(),
            position: self.position(),
            duration: self.current_duration(),
            volume: self.volume,
            repeat: self.queue.repeat(),
            shuffle: self.queue.shuffle(),
            output: self.output_info(),
        }
    }
}
