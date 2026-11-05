//! The sound card's own volume control, the one alsamixer shows. Turning it
//! leaves the samples alone, so playback stays bit-perfect at any volume, and
//! the numbers are alsamixer's: on a control with a wide dB range they follow
//! its loudness curve, on a narrow one they are plain steps.

use alsa::mixer::{MilliBel, Mixer, Selem, SelemChannelId, SelemId};
use alsa::Round;
use tracing::info;

/// alsamixer's own threshold: below this range it maps volume linearly.
const MAX_LINEAR_DB_SCALE: i64 = 2400;
/// What ALSA reports as the bottom of a range that ends in mute.
const DB_MUTE: i64 = -9_999_999;
/// Controls tried first, in this order; otherwise the first with a volume.
const PREFERRED: &[&str] = &["Master", "PCM", "Speaker", "Headphone", "Digital"];

pub struct HwVolume {
    mixer: Mixer,
    id: SelemId,
    pub control: String,
}

impl HwVolume {
    /// The volume control of the card behind `device`, if it has one.
    pub fn open(device: &str) -> Option<Self> {
        let card = card_of(device)?;
        let mixer = Mixer::new(&card, false).ok()?;
        let names: Vec<String> = mixer
            .iter()
            .filter_map(Selem::new)
            .filter(|s| s.has_playback_volume())
            .filter_map(|s| s.get_id().get_name().ok().map(str::to_string))
            .collect();
        let pick = PREFERRED
            .iter()
            .find(|p| names.iter().any(|n| n == *p))
            .map(|p| p.to_string())
            .or_else(|| names.first().cloned())?;
        info!("volume: {card} control \"{pick}\"");
        Some(Self {
            mixer,
            id: SelemId::new(&pick, 0),
            control: pick,
        })
    }

    /// 0.0 to 1.0, as alsamixer shows it.
    pub fn get(&self) -> Option<f32> {
        let _ = self.mixer.handle_events();
        let s = self.mixer.find_selem(&self.id)?;
        let (lo, hi) = s.get_playback_db_range();
        if mapped(lo, hi) {
            let db = s.get_playback_vol_db(SelemChannelId::FrontLeft).ok()?;
            Some(db_to_level(db.0, lo.0, hi.0))
        } else {
            let (min, max) = s.get_playback_volume_range();
            let raw = s.get_playback_volume(SelemChannelId::FrontLeft).ok()?;
            if max <= min {
                return None;
            }
            Some((raw - min) as f32 / (max - min) as f32)
        }
    }

    pub fn set(&self, level: f32) -> anyhow::Result<()> {
        let level = level.clamp(0.0, 1.0);
        let s = self
            .mixer
            .find_selem(&self.id)
            .ok_or_else(|| anyhow::anyhow!("mixer control {} is gone", self.control))?;
        let (lo, hi) = s.get_playback_db_range();
        if mapped(lo, hi) {
            let db = level_to_db(level, lo.0, hi.0);
            s.set_playback_db_all(MilliBel(db), Round::Floor)?;
        } else {
            let (min, max) = s.get_playback_volume_range();
            let raw = min + ((max - min) as f32 * level).round() as i64;
            s.set_playback_volume_all(raw)?;
        }
        Ok(())
    }
}

fn mapped(lo: MilliBel, hi: MilliBel) -> bool {
    hi.0 > lo.0 && hi.0 - lo.0 > MAX_LINEAR_DB_SCALE
}

/// alsa-utils' volume_mapping: loudness that sounds even across the knob.
fn db_to_level(db: i64, lo: i64, hi: i64) -> f32 {
    let norm = 10f64.powf((db - hi) as f64 / 6000.0);
    let v = if lo != DB_MUTE {
        let min = 10f64.powf((lo - hi) as f64 / 6000.0);
        (norm - min) / (1.0 - min)
    } else {
        norm
    };
    v.clamp(0.0, 1.0) as f32
}

fn level_to_db(level: f32, lo: i64, hi: i64) -> i64 {
    if level <= 0.0 {
        return lo;
    }
    let mut v = level as f64;
    if lo != DB_MUTE {
        let min = 10f64.powf((lo - hi) as f64 / 6000.0);
        v = v * (1.0 - min) + min;
    }
    if v <= 0.0 {
        return lo;
    }
    ((6000.0 * v.log10()).round() as i64 + hi).clamp(lo, hi)
}

/// The mixer for a PCM name: `hw:1,0` → `hw:1`, `plughw:CARD=DAC,DEV=0` →
/// `hw:CARD=DAC`, `default` → `default`. Anything else has no card to ask.
fn card_of(device: &str) -> Option<String> {
    if device == "default" {
        return Some("default".into());
    }
    let rest = device
        .strip_prefix("plughw:")
        .or_else(|| device.strip_prefix("hw:"))?;
    let card = rest.split(',').next()?;
    Some(format!("hw:{card}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_from_devices() {
        assert_eq!(card_of("hw:1,0").as_deref(), Some("hw:1"));
        assert_eq!(
            card_of("plughw:CARD=DAC,DEV=0").as_deref(),
            Some("hw:CARD=DAC")
        );
        assert_eq!(card_of("default").as_deref(), Some("default"));
        assert_eq!(card_of("tofile"), None);
    }

    #[test]
    fn the_curve_goes_both_ways() {
        // A typical onboard Master: -65.25 dB to 0 dB.
        let (lo, hi) = (-6525, 0);
        for pct in [0, 5, 30, 60, 85, 100] {
            let l = pct as f32 / 100.0;
            let back = db_to_level(level_to_db(l, lo, hi), lo, hi);
            assert!((back - l).abs() < 0.01, "{pct}: {back}");
        }
        assert_eq!(level_to_db(1.0, lo, hi), 0);
        assert_eq!(level_to_db(0.0, lo, hi), lo);
        // Half way on the knob is about -15 dB, as in alsamixer.
        let half = level_to_db(0.5, lo, hi);
        assert!((-1700..=-1300).contains(&half), "{half}");
    }
}
