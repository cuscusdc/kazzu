//! Ranged HTTP download into a block store, exposed to symphonia as a
//! seekable [`MediaSource`].
//!
//! One "kazzu-net" thread per stream downloads `Range: bytes=a-b` chunks
//! into 64 KiB blocks (memory bounded, farthest blocks evicted first). The
//! reader ([`HttpSource`], used by the decode worker) blocks until the bytes
//! it needs arrive and tells the downloader where it is, so a seek outside
//! the downloaded area restarts the download there. Dropped connections are
//! resumed from the last byte with exponential backoff; 401/403/410 are
//! reported as an expired URL.

use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use symphonia::core::io::MediaSource;

use super::NetContext;
use super::cache::CacheWriter;

/// Store granularity.
pub(crate) const BLOCK: u64 = 64 * 1024;
/// Bytes read from the socket per iteration.
const READ_CHUNK: usize = 16 * 1024;
/// A target this close ahead of an open connection is reached by reading
/// on instead of reconnecting.
const SKIP_AHEAD: u64 = 512 * 1024;
/// Reader poll period (also bounds how fast a cancel is noticed).
const WAIT_SLICE: Duration = Duration::from_millis(200);

/// Why a stream stopped for good.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NetFailure {
    /// 401/403/410: the (signed) URL is no longer valid.
    Expired(u16),
    /// Other HTTP error that retrying will not fix (404, 400, ...).
    Http(u16),
    /// Connection problems that outlasted every retry.
    Network(String),
}

impl std::fmt::Display for NetFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetFailure::Expired(code) => write!(f, "stream URL expired or forbidden (HTTP {code})"),
            NetFailure::Http(code) => write!(f, "HTTP error {code}"),
            NetFailure::Network(msg) => write!(f, "network error: {msg}"),
        }
    }
}

#[derive(Default)]
struct State {
    /// Total length, once known (Content-Range / Content-Length / EOF).
    len: Option<u64>,
    /// Block index → bytes, always contiguous from the block start.
    blocks: HashMap<u64, Vec<u8>>,
    mem: u64,
    /// Where the reader is.
    read_pos: u64,
    /// The reader is blocked waiting for this byte.
    need: Option<u64>,
    /// First response received (length/type/range support known).
    ready: bool,
    content_type: Option<String>,
    /// Server honours Range requests.
    ranges: bool,
    failure: Option<NetFailure>,
    cancelled: bool,
    /// The player is done with the stream (played to the end): only the
    /// holes the disk cache still needs are downloaded, then the thread ends.
    detached: bool,
    /// Total bytes received (statistics, tests).
    received: u64,
    requests: u32,
}

impl State {
    /// First byte at or after `from` that is not in the store.
    fn first_missing(&self, from: u64) -> u64 {
        let mut b = from / BLOCK;
        loop {
            let start = b * BLOCK;
            match self.blocks.get(&b) {
                None => return start,
                Some(data) => {
                    let end = start + data.len() as u64;
                    if self.len.is_some_and(|l| end >= l) {
                        return end;
                    }
                    if (data.len() as u64) < BLOCK {
                        return end;
                    }
                }
            }
            b += 1;
        }
    }

    fn available_at(&self, pos: u64) -> Option<&[u8]> {
        let data = self.blocks.get(&(pos / BLOCK))?;
        let off = (pos % BLOCK) as usize;
        (off < data.len()).then(|| &data[off..])
    }

    /// Stores `data` received at `pos`. Requests always start at a block
    /// start or at the end of a partial block, so writes append.
    fn store(&mut self, mut pos: u64, mut data: &[u8]) {
        while !data.is_empty() {
            let b = pos / BLOCK;
            let off = (pos % BLOCK) as usize;
            let room = BLOCK as usize - off;
            let take = room.min(data.len());
            let block = self.blocks.entry(b).or_default();
            if off <= block.len() {
                let new_end = off + take;
                if new_end > block.len() {
                    let skip = block.len() - off;
                    block.extend_from_slice(&data[skip..take]);
                    self.mem += (new_end - (off + skip)) as u64;
                }
            } else if block.is_empty() {
                // Cannot happen with aligned requests; drop rather than leave
                // a hole inside a block.
                self.blocks.remove(&b);
            }
            pos += take as u64;
            data = &data[take..];
        }
    }

    /// Evicts blocks far from the read position until under `limit`.
    fn evict(&mut self, limit: u64, read_ahead: u64) {
        let keep_from = self.read_pos.saturating_sub(BLOCK) / BLOCK;
        let keep_to = (self.read_pos + read_ahead) / BLOCK;
        while self.mem > limit {
            let victim = self
                .blocks
                .keys()
                .copied()
                .filter(|&b| b < keep_from || b > keep_to)
                .max_by_key(|&b| (b * BLOCK).abs_diff(self.read_pos));
            match victim {
                Some(b) => {
                    if let Some(data) = self.blocks.remove(&b) {
                        self.mem -= data.len() as u64;
                    }
                }
                None => break,
            }
        }
    }
}

/// State shared by the downloader and the reader.
pub(crate) struct Link {
    state: Mutex<State>,
    cv: Condvar,
}

impl Link {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
            cv: Condvar::new(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.lock().cancelled
    }

    /// Stops the downloader and wakes a blocked reader.
    pub fn cancel(&self) {
        self.lock().cancelled = true;
        self.cv.notify_all();
    }

    /// The player no longer reads; finish the cache file (if any) and stop.
    pub fn detach(&self) {
        self.lock().detached = true;
        self.cv.notify_all();
    }

    pub fn failure(&self) -> Option<NetFailure> {
        self.lock().failure.clone()
    }

    pub fn content_type(&self) -> Option<String> {
        self.lock().content_type.clone()
    }

    /// (bytes received, requests made), for tests and diagnostics.
    pub fn stats(&self) -> (u64, u32) {
        let st = self.lock();
        (st.received, st.requests)
    }

    fn fail(&self, failure: NetFailure) {
        let mut st = self.lock();
        if st.failure.is_none() {
            st.failure = Some(failure);
        }
        drop(st);
        self.cv.notify_all();
    }

    /// Sleeps up to `d`; returns false if cancelled meanwhile.
    fn sleep(&self, d: Duration) -> bool {
        let deadline = Instant::now() + d;
        let mut st = self.lock();
        while !st.cancelled {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return true;
            };
            st = self.cv.wait_timeout(st, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        false
    }
}

/// Request description shared with the downloader thread.
pub(crate) struct Request {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

/// Starts the downloader thread for `request`.
pub(crate) fn spawn_download(
    ctx: Arc<NetContext>,
    link: Arc<Link>,
    request: Request,
    cache: Option<CacheWriter>,
) -> io::Result<()> {
    std::thread::Builder::new()
        .name("kazzu-net".into())
        .spawn(move || Downloader::new(ctx, link, request, cache).run())?;
    Ok(())
}

/// Seekable reader over the block store. Blocks (only its own thread) until
/// data arrives, the stream ends, fails or is cancelled.
pub(crate) struct HttpSource {
    link: Arc<Link>,
    pos: u64,
    len: Option<u64>,
    seekable: bool,
}

impl HttpSource {
    /// Waits for the first response (headers). Fails on HTTP errors or
    /// when the network is unreachable after the retries.
    pub fn connect(link: Arc<Link>) -> Result<Self, NetFailure> {
        let mut st = link.lock();
        loop {
            if let Some(f) = &st.failure {
                return Err(f.clone());
            }
            if st.cancelled {
                return Err(NetFailure::Network("cancelled".into()));
            }
            if st.ready {
                break;
            }
            st = link.cv.wait_timeout(st, WAIT_SLICE).unwrap_or_else(|e| e.into_inner()).0;
        }
        let len = st.len;
        let seekable = st.ranges && len.is_some();
        drop(st);
        Ok(Self {
            link,
            pos: 0,
            len,
            seekable,
        })
    }
}

impl Read for HttpSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let link = Arc::clone(&self.link);
        let mut st = link.lock();
        st.read_pos = self.pos;
        loop {
            if st.cancelled {
                return Err(io::Error::other("stream cancelled"));
            }
            if st.len.is_some_and(|l| self.pos >= l) {
                st.need = None;
                return Ok(0);
            }
            if let Some(data) = st.available_at(self.pos) {
                let n = data.len().min(buf.len());
                buf[..n].copy_from_slice(&data[..n]);
                self.pos += n as u64;
                st.read_pos = self.pos;
                if st.need.take().is_some() {
                    link.cv.notify_all();
                }
                return Ok(n);
            }
            if let Some(f) = &st.failure {
                return Err(io::Error::other(f.to_string()));
            }
            if st.need != Some(self.pos) {
                st.need = Some(self.pos);
                link.cv.notify_all();
            }
            st = link.cv.wait_timeout(st, WAIT_SLICE).unwrap_or_else(|e| e.into_inner()).0;
        }
    }
}

impl Seek for HttpSource {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let len = self.len.or_else(|| self.link.lock().len);
        let target = match from {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
            SeekFrom::End(d) => len
                .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "stream length unknown"))?
                .checked_add_signed(d),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before start"))?;
        self.pos = target;
        let mut st = self.link.lock();
        st.read_pos = target;
        drop(st);
        self.link.cv.notify_all();
        Ok(target)
    }
}

impl MediaSource for HttpSource {
    fn is_seekable(&self) -> bool {
        self.seekable
    }

    fn byte_len(&self) -> Option<u64> {
        self.len
    }
}

// ---------------------------------------------------------------------------
// Downloader
// ---------------------------------------------------------------------------

/// An open response body.
struct Conn {
    body: Box<dyn Read + Send>,
    pos: u64,
    /// Exclusive end of what this response will deliver (`u64::MAX` = until
    /// the server closes).
    end: u64,
}

enum OpenError {
    /// Worth retrying (timeouts, resets, 429, 5xx).
    Retry(String),
    Fatal(NetFailure),
    /// 416: nothing at or after the requested position.
    PastEnd(Option<u64>),
}

/// Download speed cap.
struct Throttle {
    limit: Option<u64>,
    since: Instant,
    bytes: u64,
}

impl Throttle {
    fn new(limit: Option<u64>) -> Self {
        Self {
            limit: limit.filter(|&l| l > 0),
            since: Instant::now(),
            bytes: 0,
        }
    }

    /// Sleeps as needed after `n` bytes; returns false if cancelled.
    fn consume(&mut self, n: usize, link: &Link) -> bool {
        let Some(limit) = self.limit else {
            return true;
        };
        self.bytes += n as u64;
        let due = Duration::from_secs_f64(self.bytes as f64 / limit as f64);
        let elapsed = self.since.elapsed();
        let ok = match due.checked_sub(elapsed) {
            Some(wait) => link.sleep(wait),
            None => true,
        };
        if elapsed > Duration::from_secs(2) {
            self.since = Instant::now();
            self.bytes = 0;
        }
        ok
    }
}

struct Downloader {
    ctx: Arc<NetContext>,
    link: Arc<Link>,
    request: Request,
    cache: Option<CacheWriter>,
    conn: Option<Conn>,
    attempts: u32,
    throttle: Throttle,
    buf: Vec<u8>,
}

impl Downloader {
    fn new(ctx: Arc<NetContext>, link: Arc<Link>, request: Request, cache: Option<CacheWriter>) -> Self {
        let throttle = Throttle::new(ctx.config.max_bandwidth);
        Self {
            ctx,
            link,
            request,
            cache,
            conn: None,
            attempts: 0,
            throttle,
            buf: vec![0; READ_CHUNK],
        }
    }

    fn run(mut self) {
        while let Some(target) = self.next_target() {
            if !self.step(target) {
                break;
            }
        }
        // Dropping an incomplete cache writer deletes its partial file.
        self.conn = None;
        self.cache = None;
    }

    /// Blocks until there is something to download. `None` = stop
    /// (cancelled or failed). The thread stays alive while idle so that a
    /// backward seek into evicted data can still be served.
    fn next_target(&mut self) -> Option<u64> {
        let read_ahead = self.ctx.config.read_ahead.max(BLOCK);
        let mut st = self.link.lock();
        loop {
            if st.cancelled || st.failure.is_some() {
                return None;
            }
            if st.detached {
                let gap = match (&self.cache, st.len) {
                    (Some(cache), Some(len)) => cache.first_gap(len),
                    _ => None,
                };
                return gap;
            }
            let base = st.need.unwrap_or(st.read_pos);
            let t = st.first_missing(base);
            let at_end = st.len.is_some_and(|l| t >= l);
            if !at_end && (st.need.is_some() || t < st.read_pos.saturating_add(read_ahead)) {
                return Some(t);
            }
            if at_end
                && let (Some(cache), Some(len)) = (&self.cache, st.len)
                && let Some(gap) = cache.first_gap(len)
            {
                // Everything the player needs is here: fill the holes left
                // by seeks so the file can be cached whole. The exact gap
                // start is requested (bytes that do not line up with the
                // memory blocks only go to the cache file).
                return Some(gap);
            }
            st = self
                .link
                .cv
                .wait_timeout(st, Duration::from_millis(500))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// Downloads one read's worth toward `target`. Returns false to stop.
    fn step(&mut self, target: u64) -> bool {
        let ranges = self.link.lock().ranges;
        let reuse = self.conn.as_ref().is_some_and(|c| {
            target < c.end
                && (c.pos == target
                    || (c.pos < target && (!ranges || target - c.pos <= SKIP_AHEAD)))
        });
        if !reuse {
            self.conn = None;
            match self.open(target) {
                Ok(conn) => self.conn = Some(conn),
                Err(OpenError::Fatal(f)) => {
                    log::warn!("stream failed: {f}");
                    self.link.fail(f);
                    return false;
                }
                Err(OpenError::PastEnd(total)) => {
                    let mut st = self.link.lock();
                    st.len = Some(total.unwrap_or(target).min(st.len.unwrap_or(u64::MAX)));
                    st.ready = true;
                    drop(st);
                    self.link.cv.notify_all();
                    return true;
                }
                Err(OpenError::Retry(msg)) => return self.retry(msg),
            }
        }

        let Some(conn) = self.conn.as_mut() else {
            return true;
        };
        let max = (conn.end - conn.pos).min(READ_CHUNK as u64) as usize;
        match conn.body.read(&mut self.buf[..max]) {
            Ok(0) => {
                let pos = conn.pos;
                let promised = conn.end != u64::MAX;
                self.conn = None;
                if promised {
                    // Closed before the promised end: resume from `pos`.
                    return self.retry(format!("connection closed at byte {pos}"));
                }
                let mut st = self.link.lock();
                if st.len.is_none() {
                    st.len = Some(pos);
                }
                drop(st);
                self.link.cv.notify_all();
                true
            }
            Ok(n) => {
                let pos = conn.pos;
                conn.pos += n as u64;
                if conn.pos >= conn.end {
                    self.conn = None;
                }
                self.attempts = 0;
                let data = &self.buf[..n];
                if let Some(cache) = self.cache.as_mut() {
                    cache.write(pos, data);
                    if cache.is_complete() {
                        let content_type = self.link.content_type();
                        if let Some(cache) = self.cache.take() {
                            cache.finish(content_type.as_deref());
                        }
                    }
                }
                let cfg = &self.ctx.config;
                let limit = cfg.memory_limit.max(cfg.read_ahead + 2 * BLOCK);
                let mut st = self.link.lock();
                st.store(pos, data);
                st.received += n as u64;
                st.evict(limit, cfg.read_ahead);
                drop(st);
                self.link.cv.notify_all();
                self.throttle.consume(n, &self.link)
            }
            Err(e) => {
                self.conn = None;
                self.retry(e.to_string())
            }
        }
    }

    /// Backs off before the next attempt; false when out of retries.
    fn retry(&mut self, msg: String) -> bool {
        self.attempts += 1;
        let cfg = &self.ctx.config;
        if self.attempts > cfg.max_retries {
            log::warn!("stream: giving up after {} attempts: {msg}", self.attempts - 1);
            self.link.fail(NetFailure::Network(msg));
            return false;
        }
        let shift = (self.attempts - 1).min(16);
        let delay = cfg.retry_base.saturating_mul(1 << shift).min(cfg.retry_max);
        log::debug!("stream: retry {} in {delay:?}: {msg}", self.attempts);
        self.link.sleep(delay)
    }

    fn open(&mut self, target: u64) -> Result<Conn, OpenError> {
        let cfg = &self.ctx.config;
        let len = self.link.lock().len;
        let mut last = target.saturating_add(cfg.request_chunk.max(BLOCK)) - 1;
        if let Some(l) = len {
            last = last.min(l.saturating_sub(1));
        }
        let mut req = self.ctx.agent.get(&self.request.url);
        for (name, value) in &self.request.headers {
            if !name.eq_ignore_ascii_case("range") {
                req = req.header(name.as_str(), value.as_str());
            }
        }
        req = req.header("Range", format!("bytes={target}-{last}"));
        self.link.lock().requests += 1;

        let resp = req.call().map_err(|e| OpenError::Retry(e.to_string()))?;
        let status = resp.status().as_u16();
        let header = |name: &str| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let content_type = header("content-type");
        let content_length = header("content-length").and_then(|v| v.trim().parse::<u64>().ok());
        let content_range = header("content-range");

        let (pos, end, total, ranges) = match status {
            206 => {
                let (start, stop, total) = content_range
                    .as_deref()
                    .and_then(parse_content_range)
                    .ok_or_else(|| OpenError::Retry("206 without a valid Content-Range".into()))?;
                if start > target {
                    return Err(OpenError::Retry(format!("server answered from {start}, asked {target}")));
                }
                (start, stop + 1, total, true)
            }
            200 => {
                // Range ignored: the whole file from byte 0.
                let end = content_length.unwrap_or(u64::MAX);
                (0, end, content_length, false)
            }
            416 => {
                let total = content_range.as_deref().and_then(parse_unsatisfied_range);
                return Err(OpenError::PastEnd(total));
            }
            401 | 403 | 410 => return Err(OpenError::Fatal(NetFailure::Expired(status))),
            408 | 425 | 429 | 500..=599 => return Err(OpenError::Retry(format!("HTTP {status}"))),
            _ => return Err(OpenError::Fatal(NetFailure::Http(status))),
        };

        let mut st = self.link.lock();
        if let Some(total) = total {
            st.len = Some(total);
        }
        if !st.ready {
            st.ready = true;
            st.ranges = ranges;
            st.content_type = content_type;
        } else if !ranges {
            st.ranges = false;
        }
        let len = st.len;
        drop(st);
        self.link.cv.notify_all();
        if let (Some(cache), Some(len)) = (self.cache.as_mut(), len) {
            cache.set_len(len);
        }
        Ok(Conn {
            body: Box::new(resp.into_body().into_reader()),
            pos,
            end,
        })
    }
}

/// `bytes a-b/total` (total may be `*`).
fn parse_content_range(v: &str) -> Option<(u64, u64, Option<u64>)> {
    let rest = v.trim().strip_prefix("bytes")?.trim_start();
    let (range, total) = rest.split_once('/')?;
    let (a, b) = range.trim().split_once('-')?;
    let a = a.trim().parse().ok()?;
    let b = b.trim().parse().ok()?;
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse().ok()?),
    };
    (b >= a).then_some((a, b, total))
}

/// `bytes */total`.
fn parse_unsatisfied_range(v: &str) -> Option<u64> {
    v.trim().strip_prefix("bytes")?.trim().strip_prefix("*/")?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range_parsing() {
        assert_eq!(parse_content_range("bytes 0-99/1000"), Some((0, 99, Some(1000))));
        assert_eq!(parse_content_range("bytes 5-9/*"), Some((5, 9, None)));
        assert_eq!(parse_content_range("bytes 9-5/10"), None);
        assert_eq!(parse_unsatisfied_range("bytes */1234"), Some(1234));
    }

    #[test]
    fn store_appends_and_finds_holes() {
        let mut st = State {
            len: Some(3 * BLOCK),
            ..Default::default()
        };
        assert_eq!(st.first_missing(0), 0);
        st.store(0, &vec![1; (BLOCK + 10) as usize]);
        assert_eq!(st.first_missing(0), BLOCK + 10);
        assert_eq!(st.first_missing(2 * BLOCK + 5), 2 * BLOCK);
        // Overlapping data is not duplicated.
        st.store(BLOCK, &[2; 20]);
        assert_eq!(st.blocks[&1].len(), 20);
        assert_eq!(st.mem, BLOCK + 20);
        st.store(BLOCK + 20, &vec![3; (2 * BLOCK - 20) as usize]);
        assert_eq!(st.first_missing(0), 3 * BLOCK, "complete up to len");
        assert_eq!(st.available_at(BLOCK + 19).map(|d| d[0]), Some(2));
        assert_eq!(st.available_at(BLOCK + 20).map(|d| d[0]), Some(3));
    }

    #[test]
    fn eviction_keeps_the_read_window() {
        let mut st = State::default();
        for b in 0..10 {
            st.store(b * BLOCK, &vec![0; BLOCK as usize]);
        }
        st.read_pos = 5 * BLOCK;
        st.evict(4 * BLOCK, 2 * BLOCK);
        assert!(st.mem <= 4 * BLOCK);
        for b in 4..=7 {
            assert!(st.blocks.contains_key(&b), "block {b} near the reader is kept");
        }
    }
}
