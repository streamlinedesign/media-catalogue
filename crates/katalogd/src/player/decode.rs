//! Decoding with symphonia. One `Decoder` per file; it hands out interleaved
//! 32-bit integer samples, full scale, whatever the file holds. 16-bit FLAC
//! comes out as the original samples shifted left by 16, so nothing is lost
//! on the way to the sound card.

use anyhow::{Context, Result};
use std::fs::File;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymErr;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::{Time, TimeBase};

/// What the sound card has to be set to for this stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spec {
    pub rate: u32,
    pub channels: u32,
    /// Bits that carry information: 16 for CD, 24 for hi-res, 32 for float or unknown.
    pub bits: u32,
}

impl std::fmt::Display for Spec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} Hz · {} bit · {} ch",
            self.rate, self.bits, self.channels
        )
    }
}

pub struct Decoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    pub spec: Spec,
    buf: Option<SampleBuffer<i32>>,
    /// Frames handed out since the start (or the last seek).
    pub frames: u64,
}

pub struct Chunk<'a> {
    pub samples: &'a [i32],
    pub spec: Spec,
}

impl Decoder {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions {
                    enable_gapless: true,
                    ..Default::default()
                },
                &MetadataOptions::default(),
            )
            .with_context(|| format!("unsupported file {}", path.display()))?;
        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .context("no audio track")?;
        let params = &track.codec_params;
        let decoder = symphonia::default::get_codecs()
            .make(params, &DecoderOptions::default())
            .context("no decoder for this codec")?;
        let rate = params.sample_rate.context("unknown sample rate")?;
        let channels = params
            .channels
            .map(|c| c.count() as u32)
            .unwrap_or(2)
            .max(1);
        let bits = match params.bits_per_sample {
            Some(b) if b <= 16 => 16,
            Some(b) if b <= 24 => 24,
            _ => 32,
        };
        Ok(Self {
            track_id: track.id,
            time_base: params.time_base,
            spec: Spec {
                rate,
                channels,
                bits,
            },
            format,
            decoder,
            buf: None,
            frames: 0,
        })
    }

    /// The next block of samples, or None at the end of the file.
    pub fn next(&mut self) -> Result<Option<Chunk<'_>>> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymErr::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(None)
                }
                Err(SymErr::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                // A damaged frame: skip it, keep playing.
                Err(SymErr::DecodeError(_)) => continue,
                Err(SymErr::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(None)
                }
                Err(e) => return Err(e.into()),
            };
            let spec = *decoded.spec();
            let frames = decoded.frames();
            if frames == 0 {
                continue;
            }
            let need_new = match &self.buf {
                Some(b) => b.capacity() < frames * spec.channels.count(),
                None => true,
            };
            if need_new {
                self.buf = Some(SampleBuffer::<i32>::new(decoded.capacity() as u64, spec));
            }
            let buf = self.buf.as_mut().unwrap();
            buf.copy_interleaved_ref(decoded);
            self.frames += frames as u64;
            // The stream can change shape mid-file in theory; report what we got.
            self.spec.rate = spec.rate;
            self.spec.channels = spec.channels.count() as u32;
            return Ok(Some(Chunk {
                samples: buf.samples(),
                spec: self.spec,
            }));
        }
    }

    /// Jumps to a position. Returns where it actually landed.
    pub fn seek(&mut self, ms: u64) -> Result<u64> {
        let secs = ms / 1000;
        let frac = (ms % 1000) as f64 / 1000.0;
        let seeked = self.format.seek(
            SeekMode::Accurate,
            SeekTo::Time {
                time: Time::new(secs, frac),
                track_id: Some(self.track_id),
            },
        )?;
        self.decoder.reset();
        let landed_ms = match self.time_base {
            Some(tb) => {
                let t = tb.calc_time(seeked.actual_ts);
                t.seconds * 1000 + (t.frac * 1000.0) as u64
            }
            None => ms,
        };
        self.frames = landed_ms * self.spec.rate as u64 / 1000;
        Ok(landed_ms)
    }
}
