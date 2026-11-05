//! Stations: every playlist, and the whole library, as a radio that is always
//! on. Nothing runs in the background. A station is a schedule: its tracks in a
//! shuffled order that changes every time round, laid end to end from a fixed
//! start in 2026. What is on at any moment is arithmetic, so tuning in joins a
//! song part way through, like a real radio, and costs nothing until then.
//!
//! The order keeps the same artist from playing twice in a row where the
//! station allows it. Adding to or removing from a playlist changes its
//! schedule from then on.

use crate::db;
use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;

/// Where every schedule starts: 2026-01-01T00:00:00Z, in Unix milliseconds.
pub const EPOCH_MS: i64 = 1_767_225_600_000;

/// One track as the schedule sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub track_id: i64,
    pub artist_id: i64,
    pub duration_ms: i64,
}

/// A track's turn on a station.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Slot {
    pub track_id: i64,
    /// When it started (or starts), Unix milliseconds.
    pub start_ms: i64,
    pub duration_ms: i64,
}

impl Slot {
    pub fn end_ms(&self) -> i64 {
        self.start_ms + self.duration_ms
    }
}

/// The tracks of a station: a playlist, or `None` for the whole library.
/// Tracks without a known length cannot be scheduled and are left out.
pub fn entries(conn: &Connection, playlist: Option<i64>) -> Result<Vec<Entry>> {
    let mut st = conn.prepare(
        "SELECT t.id, t.artist_id, t.duration_ms FROM tracks t
         WHERE t.duration_ms > 0
           AND (?1 IS NULL OR t.id IN (SELECT track_id FROM playlist_tracks WHERE playlist_id = ?1))
         ORDER BY t.id",
    )?;
    let rows = st.query_map(params![playlist], |r| {
        Ok(Entry {
            track_id: r.get(0)?,
            artist_id: r.get(1)?,
            duration_ms: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The slot playing on a station at `at_ms`. `key` tells stations apart (the
/// playlist id, 0 for the library) so two stations with the same tracks still
/// play them in different orders.
pub fn slot_at(entries: &[Entry], key: i64, at_ms: i64) -> Option<Slot> {
    let total: i64 = entries.iter().map(|e| e.duration_ms).sum();
    if total <= 0 {
        return None;
    }
    let since = at_ms - EPOCH_MS;
    let cycle = since.div_euclid(total);
    let mut start = EPOCH_MS + cycle * total;
    let within = since.rem_euclid(total);
    let mut cum = 0;
    for i in order(entries, key, cycle) {
        let e = &entries[i];
        if within < cum + e.duration_ms {
            return Some(Slot {
                track_id: e.track_id,
                start_ms: start,
                duration_ms: e.duration_ms,
            });
        }
        cum += e.duration_ms;
        start += e.duration_ms;
    }
    None
}

/// The station's order for one time round. Like `round`, and the round's
/// first song is also kept off the artist that ended the round before.
fn order(entries: &[Entry], key: i64, cycle: i64) -> Vec<usize> {
    let mut idx = round(entries, key, cycle);
    let n = idx.len();
    if n < 3 {
        return idx;
    }
    // This fix never moves a round's last song, so the previous round's
    // last is what was really heard.
    let before = entries[*round(entries, key, cycle - 1).last().unwrap()].artist_id;
    let art = |idx: &[usize], k: usize| entries[idx[k]].artist_id;
    if art(&idx, 0) != before {
        return idx;
    }
    let clashes = |idx: &[usize]| {
        (art(idx, 0) == before) as usize
            + idx
                .windows(2)
                .filter(|w| entries[w[0]].artist_id == entries[w[1]].artist_id)
                .count()
    };
    let now = clashes(&idx);
    for j in 1..n - 1 {
        idx.swap(0, j);
        if clashes(&idx) < now {
            return idx;
        }
        idx.swap(0, j);
    }
    idx
}

/// One time round: shuffled, then nudged so one artist does not follow
/// itself when another can go in between.
fn round(entries: &[Entry], key: i64, cycle: i64) -> Vec<usize> {
    let n = entries.len();
    let mut idx: Vec<usize> = (0..n).collect();
    let mut rng = SplitMix(
        (key as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (cycle as u64).wrapping_add(0x5EED),
    );
    for i in (1..n).rev() {
        let j = (rng.next() % (i as u64 + 1)) as usize;
        idx.swap(i, j);
    }
    for i in 1..n {
        let prev = entries[idx[i - 1]].artist_id;
        if entries[idx[i]].artist_id != prev {
            continue;
        }
        if let Some(j) = (i + 1..n).find(|&j| entries[idx[j]].artist_id != prev) {
            idx.swap(i, j);
        }
    }
    // What is left clashes at the end, where nothing was left to swap in:
    // move each such track back to the first gap between two other artists.
    let art = |idx: &[usize], k: usize| entries[idx[k]].artist_id;
    let mut i = 1;
    while i < idx.len() {
        if art(&idx, i) != art(&idx, i - 1) {
            i += 1;
            continue;
        }
        let a = art(&idx, i);
        let item = idx.remove(i);
        let fits = (0..=idx.len())
            .find(|&p| (p == 0 || art(&idx, p - 1) != a) && (p == idx.len() || art(&idx, p) != a));
        match fits {
            Some(p) => idx.insert(p, item),
            None => {
                idx.insert(i, item);
                i += 1;
            }
        }
    }
    idx
}

struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Milliseconds since the Unix epoch, now.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// A station as the views list it: what is on now and what comes next.
#[derive(Debug, Clone, Serialize)]
pub struct OnAir {
    /// None: the whole library.
    pub playlist: Option<i64>,
    pub name: String,
    pub tracks: usize,
    pub now: Option<Playing>,
    pub next: Option<Playing>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Playing {
    pub track_id: i64,
    pub title: String,
    pub artist: String,
    pub position_ms: i64,
    pub duration_ms: i64,
}

/// Every station, the library first, with what is on at `at_ms`. Playlists
/// taken off the dial are left out.
pub fn on_air(conn: &Connection, at_ms: i64) -> Result<Vec<OnAir>> {
    let mut stations = vec![(None, "Everything".to_string())];
    stations.extend(
        db::playlists(conn)?
            .into_iter()
            .filter(|p| p.station)
            .map(|p| (Some(p.id), p.name)),
    );
    let mut out = Vec::new();
    for (playlist, name) in stations {
        let e = entries(conn, playlist)?;
        let key = playlist.unwrap_or(0);
        let describe = |slot: Option<Slot>| -> Result<Option<Playing>> {
            let Some(s) = slot else { return Ok(None) };
            Ok(db::track(conn, s.track_id)?.map(|t| Playing {
                track_id: t.id,
                title: t.title,
                artist: t.artist,
                position_ms: (at_ms - s.start_ms).max(0),
                duration_ms: s.duration_ms,
            }))
        };
        let now = slot_at(&e, key, at_ms);
        let next = now.and_then(|s| slot_at(&e, key, s.end_ms()));
        out.push(OnAir {
            playlist,
            name,
            tracks: e.len(),
            now: describe(now)?,
            next: describe(next)?.map(|mut p| {
                p.position_ms = 0;
                p
            }),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station(n: i64, artists: i64) -> Vec<Entry> {
        (1..=n)
            .map(|i| Entry {
                track_id: i,
                artist_id: i % artists,
                duration_ms: 1000 * (100 + i * 7 % 60),
            })
            .collect()
    }

    #[test]
    fn slots_follow_one_another_without_gaps() {
        let e = station(12, 4);
        let total: i64 = e.iter().map(|e| e.duration_ms).sum();
        let at = EPOCH_MS + 123_456_789;
        let s = slot_at(&e, 3, at).unwrap();
        assert!(s.start_ms <= at && at < s.end_ms());
        let mut seen = std::collections::HashSet::new();
        // From the start of a time round.
        let mut at = EPOCH_MS + 17 * total;
        // One full time round: every track exactly once, back to back.
        for _ in 0..12 {
            let s = slot_at(&e, 3, at).unwrap();
            assert_eq!(s.start_ms, at);
            assert!(seen.insert(s.track_id));
            at = s.end_ms();
        }
        assert_eq!(seen.len(), 12);
    }

    #[test]
    fn the_same_moment_gives_the_same_song() {
        let e = station(30, 5);
        let at = now_ms();
        assert_eq!(slot_at(&e, 7, at), slot_at(&e, 7, at));
        // Another station with the same tracks runs its own order.
        let a: Vec<i64> = (0..30)
            .scan(EPOCH_MS, |t, _| {
                let s = slot_at(&e, 1, *t)?;
                *t = s.end_ms();
                Some(s.track_id)
            })
            .collect();
        let b: Vec<i64> = (0..30)
            .scan(EPOCH_MS, |t, _| {
                let s = slot_at(&e, 2, *t)?;
                *t = s.end_ms();
                Some(s.track_id)
            })
            .collect();
        assert_ne!(a, b);
    }

    #[test]
    fn an_artist_does_not_follow_itself_when_it_can_be_helped() {
        let e = station(40, 4);
        let mut last = None;
        for cycle in 0..20 {
            let o = order(&e, 9, cycle);
            for w in o.windows(2) {
                assert_ne!(e[w[0]].artist_id, e[w[1]].artist_id, "cycle {cycle}");
            }
            if let Some(l) = last {
                assert_ne!(
                    e[o[0]].artist_id, l,
                    "round {cycle} starts on the artist before it"
                );
            }
            last = Some(e[*o.last().unwrap()].artist_id);
        }
    }

    #[test]
    fn an_empty_station_has_nothing_on() {
        assert_eq!(slot_at(&[], 0, now_ms()), None);
    }
}
