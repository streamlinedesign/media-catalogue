//! Stations as streams anyone on the network can play:
//!
//!   mpv http://127.0.0.1:7300/stations/chill.wav
//!
//! Each listener gets its own thread that decodes the station's schedule from
//! the moment they connect, in real time, as one endless WAV at 44.1 kHz,
//! 16 bit, stereo. Files at other rates are resampled for the stream (playback
//! on the sound card stays bit-perfect; this is only the network copy). A
//! station nobody listens to does no work at all.

use crate::player::decode::Decoder;
use crate::Shared;
use bytes::Bytes;
use katalog_core::{db, stations};
use rubato::{FftFixedIn, Resampler};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{info, warn};

pub const RATE: u32 = 44_100;
const CHANNELS: usize = 2;
/// How far ahead of real time the stream may run, so players buffer a little.
const LEAD: Duration = Duration::from_secs(2);

/// The station a URL names: "everything" (or "library"), a playlist id, or a
/// playlist name, without the `.wav`.
pub fn resolve(
    conn: &rusqlite::Connection,
    name: &str,
) -> anyhow::Result<Option<(Option<i64>, String)>> {
    let name = name.strip_suffix(".wav").unwrap_or(name);
    if name.eq_ignore_ascii_case("everything") || name.eq_ignore_ascii_case("library") {
        return Ok(Some((None, "Everything".into())));
    }
    for p in db::playlists(conn)? {
        if p.id.to_string() == name || p.name.eq_ignore_ascii_case(name) {
            return Ok(Some((Some(p.id), p.name)));
        }
    }
    Ok(None)
}

/// Starts a listener's thread; the receiver yields the WAV bytes.
pub fn listen(state: Shared, playlist: Option<i64>, name: String) -> mpsc::Receiver<Bytes> {
    let (tx, rx) = mpsc::channel(8);
    std::thread::Builder::new()
        .name(format!("on air: {name}"))
        .spawn(move || {
            info!("listener tuned in to {name}");
            if tx.blocking_send(Bytes::from(wav_header())).is_ok() {
                if let Err(e) = run(&state, playlist, &tx) {
                    warn!("{name}: {e:#}");
                }
            }
            info!("listener left {name}");
        })
        .ok();
    rx
}

fn run(state: &Shared, playlist: Option<i64>, tx: &mpsc::Sender<Bytes>) -> anyhow::Result<()> {
    let key = playlist.unwrap_or(0);
    let started = Instant::now();
    let mut sent: u64 = 0;
    let mut at = stations::now_ms();
    let mut first = true;
    loop {
        let (slot, path) = {
            let conn = state.db.lock();
            let entries = stations::entries(&conn, playlist)?;
            let Some(slot) = stations::slot_at(&entries, key, at) else {
                anyhow::bail!("nothing to play");
            };
            let track = db::track(&conn, slot.track_id)?;
            (slot, track.map(|t| t.path))
        };
        at = slot.end_ms();
        let Some(path) = path else { continue };
        let mut dec = match Decoder::open(Path::new(&path)) {
            Ok(d) => d,
            Err(e) => {
                warn!("{path}: {e:#}");
                // Keep the schedule: silence for the slot.
                let frames = slot.duration_ms as u64 * RATE as u64 / 1000;
                sent += send_silence(tx, frames, &started, sent)?;
                continue;
            }
        };
        if first {
            let into = (stations::now_ms() - slot.start_ms).max(0) as u64;
            if into > 500 {
                dec.seek(into)?;
            }
            first = false;
        }
        let mut conv = Convert::new(dec.spec.rate)?;
        while let Some(chunk) = dec.next()? {
            let out = conv.push(chunk.samples, chunk.spec.channels as usize);
            sent += pace_and_send(tx, out, &started, sent)?;
        }
        let out = conv.finish();
        sent += pace_and_send(tx, out, &started, sent)?;
    }
}

/// Waits until the stream is no more than `LEAD` ahead of real time, then sends.
fn pace_and_send(
    tx: &mpsc::Sender<Bytes>,
    pcm: Vec<u8>,
    started: &Instant,
    sent: u64,
) -> anyhow::Result<u64> {
    if pcm.is_empty() {
        return Ok(0);
    }
    let ahead = Duration::from_secs_f64(sent as f64 / RATE as f64);
    let now = started.elapsed();
    if ahead > now + LEAD {
        std::thread::sleep(ahead - now - LEAD);
    }
    let frames = (pcm.len() / (2 * CHANNELS)) as u64;
    tx.blocking_send(Bytes::from(pcm))
        .map_err(|_| anyhow::anyhow!("listener gone"))?;
    Ok(frames)
}

fn send_silence(
    tx: &mpsc::Sender<Bytes>,
    frames: u64,
    started: &Instant,
    mut sent: u64,
) -> anyhow::Result<u64> {
    let mut left = frames;
    while left > 0 {
        let n = left.min(RATE as u64 / 10);
        sent += pace_and_send(tx, vec![0; n as usize * 2 * CHANNELS], started, sent)?;
        left -= n;
    }
    Ok(frames)
}

/// Any rate and channel count in, 44.1 kHz 16-bit stereo bytes out.
struct Convert {
    rs: Option<FftFixedIn<f32>>,
    pending: [Vec<f32>; CHANNELS],
}

impl Convert {
    fn new(rate: u32) -> anyhow::Result<Self> {
        let rs = if rate == RATE {
            None
        } else {
            Some(FftFixedIn::new(
                rate as usize,
                RATE as usize,
                1024,
                2,
                CHANNELS,
            )?)
        };
        Ok(Self {
            rs,
            pending: [Vec::new(), Vec::new()],
        })
    }

    fn push(&mut self, samples: &[i32], channels: usize) -> Vec<u8> {
        let ch = channels.max(1);
        for frame in samples.chunks_exact(ch) {
            let l = frame[0] as f32 / 2_147_483_648.0;
            let r = if ch > 1 {
                frame[1] as f32 / 2_147_483_648.0
            } else {
                l
            };
            self.pending[0].push(l);
            self.pending[1].push(r);
        }
        self.drain(false)
    }

    fn finish(&mut self) -> Vec<u8> {
        self.drain(true)
    }

    fn drain(&mut self, last: bool) -> Vec<u8> {
        let Some(rs) = self.rs.as_mut() else {
            let out = interleave16(&self.pending[0], &self.pending[1]);
            self.pending[0].clear();
            self.pending[1].clear();
            return out;
        };
        let mut out = Vec::new();
        loop {
            let need = rs.input_frames_next();
            if self.pending[0].len() < need {
                break;
            }
            let block = [&self.pending[0][..need], &self.pending[1][..need]];
            if let Ok(w) = rs.process(&block, None) {
                out.extend(interleave16(&w[0], &w[1]));
            }
            self.pending[0].drain(..need);
            self.pending[1].drain(..need);
        }
        if last && !self.pending[0].is_empty() {
            let block = [&self.pending[0][..], &self.pending[1][..]];
            if let Ok(w) = rs.process_partial(Some(&block), None) {
                out.extend(interleave16(&w[0], &w[1]));
            }
            self.pending[0].clear();
            self.pending[1].clear();
        }
        out
    }
}

fn interleave16(l: &[f32], r: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(l.len() * 4);
    for (a, b) in l.iter().zip(r) {
        for s in [a, b] {
            let v = (s * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// A WAV header for a stream with no end: the sizes say "as long as it goes".
fn wav_header() -> Vec<u8> {
    let mut h = Vec::with_capacity(44);
    let byte_rate = RATE * CHANNELS as u32 * 2;
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&u32::MAX.to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes()); // PCM
    h.extend_from_slice(&(CHANNELS as u16).to_le_bytes());
    h.extend_from_slice(&RATE.to_le_bytes());
    h.extend_from_slice(&byte_rate.to_le_bytes());
    h.extend_from_slice(&((CHANNELS * 2) as u16).to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&u32::MAX.to_le_bytes());
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_is_44_bytes_of_pcm_wav() {
        let h = wav_header();
        assert_eq!(h.len(), 44);
        assert_eq!(&h[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(h[24..28].try_into().unwrap()), 44_100);
    }

    #[test]
    fn mono_becomes_stereo_and_cd_passes_straight() {
        let mut c = Convert::new(44_100).unwrap();
        let out = c.push(&[i32::MAX / 2, i32::MIN / 2], 1);
        // Two frames, both channels the same.
        assert_eq!(out.len(), 8);
        assert_eq!(out[0..2], out[2..4]);
        assert_eq!(i16::from_le_bytes([out[0], out[1]]), 16384);
    }

    #[test]
    fn other_rates_come_out_at_44_1() {
        let mut c = Convert::new(96_000).unwrap();
        let second: Vec<i32> = (0..96_000)
            .flat_map(|i| {
                let v = ((i as f32 * 0.01).sin() * 1e9) as i32;
                [v, v]
            })
            .collect();
        let mut out = c.push(&second, 2);
        out.extend(c.finish());
        let frames = out.len() / 4;
        assert!((43_000..=46_000).contains(&frames), "{frames}");
    }
}
