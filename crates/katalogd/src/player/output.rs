//! ALSA output. Opens the device at the file's own rate and depth, so with an
//! `hw:` device the samples reach the DAC exactly as they are in the file.
//! If the card cannot do what the file needs, the `plughw:` twin of the device
//! converts, and the status says playback is not bit-perfect right now.

use super::decode::Spec;
use alsa::pcm::{Access, Format, HwParams, PCM};
use alsa::{Direction, ValueOr};
use anyhow::{Context, Result};
use serde::Serialize;
use tracing::{info, warn};

pub struct Sink {
    pcm: PCM,
    format: Format,
    pub spec: Spec,
    pub device: String,
    pub bit_perfect: bool,
    can_pause: bool,
    scratch16: Vec<i16>,
    scratch32: Vec<i32>,
}

impl Sink {
    /// Opens `device` for `spec`. Falls back to conversion when the card cannot
    /// run at that rate or depth.
    pub fn open(device: &str, spec: Spec) -> Result<Self> {
        match Self::open_exact(device, spec) {
            Ok(s) => Ok(s),
            Err(e) => {
                if let Some(plug) = plug_twin(device) {
                    warn!("{device} cannot do {spec} ({e:#}); converting through {plug}");
                    let mut s = Self::open_exact(&plug, spec)
                        .with_context(|| format!("open {plug} for {spec}"))?;
                    s.bit_perfect = false;
                    s.device = device.to_string();
                    Ok(s)
                } else {
                    Err(e).with_context(|| format!("open {device} for {spec}"))
                }
            }
        }
    }

    fn open_exact(device: &str, spec: Spec) -> Result<Self> {
        let pcm = PCM::new(device, Direction::Playback, false)
            .with_context(|| format!("open {device}"))?;
        let (format, can_pause) = {
            let hwp = HwParams::any(&pcm)?;
            hwp.set_access(Access::RWInterleaved)?;
            hwp.set_channels(spec.channels)?;
            hwp.set_rate(spec.rate, ValueOr::Nearest)?;
            let got = hwp.get_rate()?;
            if got != spec.rate {
                anyhow::bail!("card runs at {got} Hz, file is {} Hz", spec.rate);
            }
            // The container the card takes without losing bits.
            let candidates: &[Format] = if spec.bits <= 16 {
                &[Format::S16LE, Format::S32LE, Format::S24LE]
            } else {
                &[Format::S32LE, Format::S24LE]
            };
            let format = candidates
                .iter()
                .copied()
                .find(|f| hwp.test_format(*f).is_ok())
                .with_context(|| format!("card has no format for {} bit", spec.bits))?;
            hwp.set_format(format)?;
            // Half a second of buffer: enough for a busy machine, short enough to pause.
            hwp.set_buffer_time_near(500_000, ValueOr::Nearest)?;
            hwp.set_period_time_near(50_000, ValueOr::Nearest)?;
            pcm.hw_params(&hwp)?;
            (format, hwp.can_pause())
        };
        {
            let hwp = pcm.hw_params_current()?;
            let swp = pcm.sw_params_current()?;
            // Start when the buffer is nearly full, not on the first sample.
            swp.set_start_threshold(hwp.get_buffer_size()? - hwp.get_period_size()?)?;
            pcm.sw_params(&swp)?;
        }
        let bit_perfect = device.starts_with("hw:");
        info!(
            "{device}: {spec} as {format:?}{}",
            if bit_perfect { ", bit-perfect" } else { "" }
        );
        Ok(Self {
            pcm,
            format,
            spec,
            device: device.to_string(),
            bit_perfect,
            can_pause,
            scratch16: Vec::new(),
            scratch32: Vec::new(),
        })
    }

    /// Writes interleaved full-scale 32-bit samples, blocking until they fit.
    pub fn write(&mut self, samples: &[i32]) -> Result<()> {
        let ch = self.spec.channels as usize;
        let frames = samples.len() / ch;
        let mut written = 0usize;
        while written < frames {
            let res = match self.format {
                Format::S16LE => {
                    self.scratch16.clear();
                    self.scratch16
                        .extend(samples[written * ch..].iter().map(|s| (s >> 16) as i16));
                    self.pcm.io_i16()?.writei(&self.scratch16)
                }
                Format::S24LE => {
                    self.scratch32.clear();
                    self.scratch32
                        .extend(samples[written * ch..].iter().map(|s| s >> 8));
                    self.pcm.io_i32()?.writei(&self.scratch32)
                }
                _ => self.pcm.io_i32()?.writei(&samples[written * ch..]),
            };
            match res {
                Ok(n) => written += n,
                Err(e) => {
                    // Underrun or suspend: recover and keep going.
                    self.pcm.try_recover(e, true).context("sound card error")?;
                }
            }
        }
        Ok(())
    }

    /// Frames written but not yet heard.
    pub fn delay_frames(&self) -> u64 {
        self.pcm.delay().map(|d| d.max(0) as u64).unwrap_or(0)
    }

    pub fn pause(&mut self) {
        if self.can_pause {
            let _ = self.pcm.pause(true);
        } else {
            // Drop what is buffered; the player rewinds by that much on resume.
            let _ = self.pcm.drop();
        }
    }

    pub fn resume(&mut self) {
        if self.can_pause {
            let _ = self.pcm.pause(false);
        } else {
            let _ = self.pcm.prepare();
        }
    }

    /// True when `pause` throws away the buffered audio instead of holding it.
    pub fn pause_drops(&self) -> bool {
        !self.can_pause
    }

    /// Throws away what is buffered, for a seek.
    pub fn flush(&mut self) {
        let _ = self.pcm.drop();
        let _ = self.pcm.prepare();
    }

    /// Plays out what is buffered, then stops. For the end of the queue.
    pub fn drain(&mut self) {
        let _ = self.pcm.drain();
    }
}

/// `hw:1,0` → `plughw:1,0`; `hw:CARD=DAC` → `plughw:CARD=DAC`. Anything else: none.
fn plug_twin(device: &str) -> Option<String> {
    device
        .strip_prefix("hw:")
        .map(|rest| format!("plughw:{rest}"))
}

#[derive(Debug, Clone, Serialize)]
pub struct Device {
    pub name: String,
    pub description: String,
}

/// Every `hw:` playback device ALSA knows about, plus `default`.
pub fn devices() -> Vec<Device> {
    let mut out = vec![Device {
        name: "default".into(),
        description: "ALSA default (dmix or PipeWire, converts to a fixed rate)".into(),
    }];
    if let Ok(hints) = alsa::device_name::HintIter::new_str(None, "pcm") {
        for h in hints {
            let Some(name) = h.name else { continue };
            if !name.starts_with("hw:") {
                continue;
            }
            if matches!(h.direction, Some(Direction::Capture)) {
                continue;
            }
            let description = h
                .desc
                .unwrap_or_default()
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
            out.push(Device { name, description });
        }
    }
    out
}

/// Picks a card for `--device auto`: the first hardware output that is not HDMI,
/// otherwise `default`.
pub fn auto_device() -> String {
    let list = devices();
    let pick = list
        .iter()
        .skip(1)
        .find(|d| !d.description.to_ascii_lowercase().contains("hdmi"))
        .or_else(|| list.get(1));
    match pick {
        Some(d) => {
            info!("sound device: {} ({})", d.name, d.description);
            d.name.clone()
        }
        None => {
            info!("no hardware output found, using ALSA default");
            "default".into()
        }
    }
}
