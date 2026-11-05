//! A plain spectrum for the window: a handful of bands, 0.0 to 1.0, worked out
//! from the samples on their way to the card and published when they are heard,
//! not when they are written, so the bars keep time with the sound.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Bars in the spectrum.
pub const BANDS: usize = 32;
const FFT: usize = 2048;
/// Analyses per second.
const RATE_HZ: u32 = 30;
/// Below this the bar is empty.
const FLOOR_DB: f32 = -70.0;

pub type Shared = Arc<Mutex<Vec<f32>>>;

pub fn shared() -> Shared {
    Arc::new(Mutex::new(vec![0.0; BANDS]))
}

pub struct Meter {
    out: Shared,
    /// The last FFT frames of mono signal, -1.0 to 1.0.
    window: VecDeque<f32>,
    since: usize,
    /// Frames written to the card since the last reset.
    written: u64,
    /// Waiting to be heard: (written frame count, bands).
    pending: VecDeque<(u64, Vec<f32>)>,
    hann: Vec<f32>,
    edges: Vec<(usize, usize)>,
    rate: u32,
}

impl Meter {
    pub fn new(out: Shared) -> Self {
        let hann = (0..FFT)
            .map(|i| {
                let x = std::f32::consts::PI * 2.0 * i as f32 / (FFT - 1) as f32;
                0.5 - 0.5 * x.cos()
            })
            .collect();
        Self {
            out,
            window: VecDeque::with_capacity(FFT),
            since: 0,
            written: 0,
            pending: VecDeque::new(),
            hann,
            edges: Vec::new(),
            rate: 0,
        }
    }

    /// Takes interleaved full-scale samples just written to the card.
    pub fn feed(&mut self, samples: &[i32], channels: usize, rate: u32) {
        if rate != self.rate {
            self.rate = rate;
            self.edges = band_edges(rate);
        }
        let hop = (rate / RATE_HZ).max(1) as usize;
        let ch = channels.max(1);
        for frame in samples.chunks_exact(ch) {
            let sum: f32 = frame.iter().map(|s| *s as f32).sum();
            if self.window.len() == FFT {
                self.window.pop_front();
            }
            self.window.push_back(sum / ch as f32 / i32::MAX as f32);
            self.written += 1;
            self.since += 1;
            if self.since >= hop && self.window.len() == FFT {
                self.since = 0;
                let bands = self.analyse();
                self.pending.push_back((self.written, bands));
                // Half a second of buffer at 30 a second; never grow without end.
                if self.pending.len() > 120 {
                    self.pending.pop_front();
                }
            }
        }
    }

    /// Publishes what the card is playing now, `delay` frames behind what was written.
    pub fn heard(&mut self, delay: u64) {
        let now = self.written.saturating_sub(delay);
        let mut latest = None;
        while let Some((at, _)) = self.pending.front() {
            if *at > now {
                break;
            }
            latest = self.pending.pop_front().map(|(_, b)| b);
        }
        if let Some(b) = latest {
            self.set(b);
        }
    }

    /// Silence: a seek, a pause, a stop, a new sink.
    pub fn reset(&mut self) {
        self.window.clear();
        self.pending.clear();
        self.since = 0;
        self.written = 0;
        self.set(vec![0.0; BANDS]);
    }

    fn set(&self, bands: Vec<f32>) {
        if let Ok(mut o) = self.out.lock() {
            *o = bands;
        }
    }

    fn analyse(&self) -> Vec<f32> {
        let mut re: Vec<f32> = self
            .window
            .iter()
            .zip(&self.hann)
            .map(|(s, w)| s * w)
            .collect();
        let mut im = vec![0.0f32; FFT];
        fft(&mut re, &mut im);
        // A full-scale sine through a Hann window peaks at about FFT/4.
        let norm = (FFT / 4) as f32;
        self.edges
            .iter()
            .map(|&(lo, hi)| {
                let peak = (lo..hi)
                    .map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt())
                    .fold(0.0f32, f32::max);
                let db = 20.0 * (peak / norm).max(1e-9).log10();
                ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
            })
            .collect()
    }
}

/// FFT bins for each band, spaced evenly in pitch from 40 Hz to 16 kHz.
fn band_edges(rate: u32) -> Vec<(usize, usize)> {
    let (lo, hi) = (40.0f32, 16_000.0f32.min(rate as f32 / 2.0));
    let bin = rate as f32 / FFT as f32;
    (0..BANDS)
        .map(|i| {
            let f0 = lo * (hi / lo).powf(i as f32 / BANDS as f32);
            let f1 = lo * (hi / lo).powf((i + 1) as f32 / BANDS as f32);
            let a = ((f0 / bin) as usize).clamp(1, FFT / 2 - 1);
            let b = ((f1 / bin).ceil() as usize).clamp(a + 1, FFT / 2);
            (a, b)
        })
        .collect()
}

/// In-place radix-2 FFT. `re.len()` is a power of two.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let nr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = nr;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tone_lights_its_band() {
        let out = shared();
        let mut m = Meter::new(out.clone());
        let rate = 44_100;
        let samples: Vec<i32> = (0..rate)
            .flat_map(|i| {
                let v = (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / rate as f32).sin();
                let s = (v * 0.5 * i32::MAX as f32) as i32;
                [s, s]
            })
            .collect();
        m.feed(&samples, 2, rate);
        m.heard(0);
        let bands = out.lock().unwrap().clone();
        let loudest = bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        let edges = band_edges(rate);
        let bin = rate as f32 / FFT as f32;
        let (lo, hi) = edges[loudest];
        assert!(
            (lo as f32 * bin) <= 1000.0 && 1000.0 <= (hi as f32 * bin),
            "{loudest}"
        );
        assert!(bands[loudest] > 0.8);
        assert!(bands[0] < 0.3);
    }

    #[test]
    fn nothing_is_published_before_it_is_heard() {
        let out = shared();
        let mut m = Meter::new(out.clone());
        let samples = vec![i32::MAX / 2; 44_100 * 2];
        m.feed(&samples, 2, 44_100);
        m.heard(44_100);
        assert!(out.lock().unwrap().iter().all(|b| *b == 0.0));
    }
}
