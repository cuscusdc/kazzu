//! Symphonia-based file decoder with sample-accurate seeking.

use std::fs::File;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use kazzu_core::{AudioFormat, Codec, Error, Result, StreamSource};
use symphonia::core::codecs::audio::well_known::{
    CODEC_ID_AAC, CODEC_ID_ALAC, CODEC_ID_FLAC, CODEC_ID_MP1, CODEC_ID_MP2, CODEC_ID_MP3,
    CODEC_ID_OPUS, CODEC_ID_PCM_MULAW, CODEC_ID_PCM_S32LE, CODEC_ID_VORBIS,
};
use symphonia::core::codecs::audio::{AudioCodecId, AudioDecoder, AudioDecoderOptions};
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{Time, TimeBase, Timestamp};

/// Consecutive undecodable packets tolerated before giving up on a stream.
const MAX_CONSECUTIVE_DECODE_ERRORS: u32 = 64;

/// Codec registry: every symphonia codec plus libopus (feature `opus`).
fn codecs() -> &'static CodecRegistry {
    static REGISTRY: OnceLock<CodecRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut registry = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut registry);
        #[cfg(feature = "opus")]
        registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
        registry
    })
}

/// PCM layout produced by a decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpec {
    pub sample_rate: u32,
    pub channels: u16,
    /// Bits per sample of the source (lossless/PCM only).
    pub bits_per_sample: Option<u32>,
}

/// An open, decodable audio stream.
pub struct TrackDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    spec: SourceSpec,
    duration: Option<Duration>,
    audio_format: AudioFormat,
    /// After a seek: drop decoded frames before this timestamp.
    skip_until: Option<Timestamp>,
    scratch: Vec<f32>,
}

impl TrackDecoder {
    /// Opens a local stream. [`StreamSource::Url`] is not opened here: the
    /// engine opens remote streams asynchronously, on a network worker
    /// thread, so that it never blocks on the network.
    pub fn open(source: &StreamSource) -> Result<Self> {
        match source {
            StreamSource::File { path } => Self::open_file(path),
            StreamSource::Deferred | StreamSource::Url { .. } => Err(Error::Unsupported(
                "URL streams are opened asynchronously by the engine".into(),
            )),
        }
    }

    /// Probes any symphonia media source (used for HTTP streams). `label`
    /// names the source in errors.
    pub(crate) fn open_media(source: Box<dyn MediaSource>, hint: &Hint, label: &str) -> Result<Self> {
        let mss = MediaSourceStream::new(source, Default::default());
        let format = symphonia::default::get_probe()
            .probe(hint, mss, FormatOptions::default(), MetadataOptions::default())
            .map_err(|e| Error::Decode(format!("{label}: {e}")))?;
        Self::from_reader(format)
    }

    /// Reconciles the container duration with the one from the track
    /// metadata. Fragmented MP4 (DASH, as served by YouTube) only declares
    /// its first fragment, so a much longer metadata duration wins; that
    /// also lets seeks reach past the first fragment.
    pub fn reconcile_duration(&mut self, metadata: Option<Duration>) {
        self.duration = match (self.duration, metadata) {
            (Some(d), Some(m)) if m > d + Duration::from_secs(2) => Some(m),
            (None, m) => m,
            (d, _) => d,
        };
    }

    pub fn open_file(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        let format = symphonia::default::get_probe()
            .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
            .map_err(|e| Error::Decode(format!("{}: {e}", path.display())))?;
        Self::from_reader(format)
    }

    fn from_reader(format: Box<dyn FormatReader>) -> Result<Self> {
        let track = format
            .default_track(TrackType::Audio)
            .ok_or_else(|| Error::Decode("no audio track".into()))?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| Error::Decode("missing audio codec parameters".into()))?;

        let decoder = codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default())
            .map_err(|e| Error::Unsupported(format!("codec: {e}")))?;

        let sample_rate = params
            .sample_rate
            .ok_or_else(|| Error::Decode("unknown sample rate".into()))?;
        let channels = params
            .channels
            .as_ref()
            .map(|c| c.count() as u16)
            .ok_or_else(|| Error::Decode("unknown channel layout".into()))?;
        let bits_per_sample = params.bits_per_sample;

        let time_base = track.time_base;
        let duration = track
            .num_frames
            .map(|n| frames_to_duration(n, sample_rate))
            .or_else(|| {
                let tb = time_base?;
                let time = tb.calc_duration(track.duration?)?;
                time_to_duration(time)
            });

        let codec = codec_from_id(params.codec);
        let lossless = codec.is_lossless();
        let audio_format = AudioFormat {
            codec,
            sample_rate: Some(sample_rate),
            bit_depth: if lossless {
                bits_per_sample.and_then(|b| u8::try_from(b).ok())
            } else {
                None
            },
            channels: Some(channels),
            bitrate_kbps: None,
        };

        Ok(Self {
            track_id: track.id,
            format,
            decoder,
            time_base,
            spec: SourceSpec {
                sample_rate,
                channels,
                bits_per_sample,
            },
            duration,
            audio_format,
            skip_until: None,
            scratch: Vec::new(),
        })
    }

    pub fn spec(&self) -> SourceSpec {
        self.spec
    }

    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    pub fn audio_format(&self) -> &AudioFormat {
        &self.audio_format
    }

    /// Sample-accurate seek. Returns the position decoding resumes at.
    pub fn seek(&mut self, position: Duration) -> Result<Duration> {
        let position = match self.duration {
            Some(d) if position > d => d,
            _ => position,
        };
        let time = Time::try_from_nanos_u128(position.as_nanos())
            .ok_or_else(|| Error::InvalidArgument("seek position out of range".into()))?;
        let seeked = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| Error::Decode(format!("seek failed: {e}")))?;
        self.decoder.reset();
        self.skip_until = Some(seeked.required_ts);
        Ok(position)
    }

    /// Decodes the next packet and appends its interleaved samples to `out`.
    /// Returns `Ok(false)` at end of stream.
    pub fn decode_next(&mut self, out: &mut Vec<f32>) -> Result<bool> {
        let mut errors = 0;
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => return Ok(false),
                Err(SymError::ResetRequired) => {
                    // Track list changed (e.g. chained Ogg): rebuild the decoder.
                    self.rebuild_decoder()?;
                    continue;
                }
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(false);
                }
                Err(e) => return Err(Error::Decode(e.to_string())),
            };
            if packet.track_id != self.track_id {
                continue;
            }

            let pts = packet.pts;
            let buf = match self.decoder.decode(&packet) {
                Ok(buf) => buf,
                Err(SymError::DecodeError(msg)) => {
                    errors += 1;
                    log::debug!("skipping corrupt packet: {msg}");
                    if errors >= MAX_CONSECUTIVE_DECODE_ERRORS {
                        return Err(Error::Decode(format!("too many decode errors: {msg}")));
                    }
                    continue;
                }
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(false);
                }
                Err(e) => return Err(Error::Decode(e.to_string())),
            };

            let frames = buf.frames();
            if frames == 0 {
                continue;
            }
            let buf_channels = buf.spec().channels().count();
            buf.copy_to_vec_interleaved(&mut self.scratch);

            // Drop frames that precede an accurate-seek target.
            let mut skip_frames = 0usize;
            if let Some(target) = self.skip_until {
                let ahead = target.get() - pts.get();
                if ahead > 0 {
                    skip_frames = self.ts_delta_to_frames(ahead as u64).min(frames as u64) as usize;
                }
                if skip_frames < frames {
                    self.skip_until = None;
                }
            }
            if skip_frames >= frames {
                continue;
            }

            let src = &self.scratch[skip_frames * buf_channels..];
            append_remapped(src, buf_channels, usize::from(self.spec.channels), out);
            return Ok(true);
        }
    }

    fn rebuild_decoder(&mut self) -> Result<()> {
        let track = self
            .format
            .default_track(TrackType::Audio)
            .ok_or_else(|| Error::Decode("no audio track after reset".into()))?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| Error::Decode("missing codec parameters after reset".into()))?;
        self.decoder = codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default())
            .map_err(|e| Error::Unsupported(format!("codec: {e}")))?;
        self.track_id = track.id;
        self.time_base = track.time_base;
        Ok(())
    }

    /// Converts a timestamp difference (track time base) into frames.
    fn ts_delta_to_frames(&self, delta: u64) -> u64 {
        match self.time_base {
            Some(tb) => {
                let numer = u128::from(tb.numer.get());
                let denom = u128::from(tb.denom.get());
                let frames = u128::from(delta) * numer * u128::from(self.spec.sample_rate) / denom;
                frames as u64
            }
            None => delta,
        }
    }
}

/// Appends `src` (interleaved, `from` channels) to `out` as `to` channels.
/// Mono is duplicated to every output channel; otherwise extra channels are
/// dropped and missing ones are zero-filled.
pub(crate) fn append_remapped(src: &[f32], from: usize, to: usize, out: &mut Vec<f32>) {
    if from == to {
        out.extend_from_slice(src);
        return;
    }
    for frame in src.chunks_exact(from) {
        for ch in 0..to {
            let sample = if from == 1 {
                frame[0]
            } else {
                frame.get(ch).copied().unwrap_or(0.0)
            };
            out.push(sample);
        }
    }
}

/// Rounds up to the next nanosecond so that converting back with a floor
/// (as symphonia's `calc_timestamp` does) yields exactly `frames` again.
pub(crate) fn frames_to_duration(frames: u64, rate: u32) -> Duration {
    let rate = u64::from(rate.max(1));
    let secs = frames / rate;
    let rem = frames % rate;
    Duration::new(secs, (rem * 1_000_000_000).div_ceil(rate) as u32)
}

#[cfg(test)]
pub(crate) fn duration_to_frames(duration: Duration, rate: u32) -> u64 {
    (duration.as_nanos() * u128::from(rate) / 1_000_000_000) as u64
}

fn time_to_duration(time: Time) -> Option<Duration> {
    u64::try_from(time.as_nanos()).ok().map(Duration::from_nanos)
}

fn codec_from_id(id: AudioCodecId) -> Codec {
    match id {
        CODEC_ID_FLAC => Codec::Flac,
        CODEC_ID_ALAC => Codec::Alac,
        CODEC_ID_MP3 | CODEC_ID_MP2 | CODEC_ID_MP1 => Codec::Mp3,
        CODEC_ID_AAC => Codec::Aac,
        CODEC_ID_VORBIS => Codec::Vorbis,
        CODEC_ID_OPUS => Codec::Opus,
        // Well-known PCM ids form a contiguous block.
        other if (CODEC_ID_PCM_S32LE..=CODEC_ID_PCM_MULAW).contains(&other) => Codec::Pcm,
        other => Codec::Other(format!("{other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_duration_conversions() {
        assert_eq!(frames_to_duration(44_100, 44_100), Duration::from_secs(1));
        assert_eq!(frames_to_duration(22_050, 44_100), Duration::from_millis(500));
        assert_eq!(duration_to_frames(Duration::from_millis(250), 48_000), 12_000);
        for rate in [44_100, 48_000, 88_200, 96_000, 192_000] {
            for frames in [1u64, 54_444, 1_234_567, 9_999_999] {
                let d = frames_to_duration(frames, rate);
                assert_eq!(duration_to_frames(d, rate), frames, "{frames} @ {rate}");
            }
        }
    }

    #[test]
    fn remap_mono_to_stereo() {
        let mut out = Vec::new();
        append_remapped(&[0.1, 0.2], 1, 2, &mut out);
        assert_eq!(out, vec![0.1, 0.1, 0.2, 0.2]);
    }
}

#[cfg(all(test, feature = "opus"))]
mod opus_tests {
    use super::*;

    #[test]
    fn opus_decoder_is_registered() {
        assert!(codecs().get_audio_decoder(CODEC_ID_OPUS).is_some());
    }
}
