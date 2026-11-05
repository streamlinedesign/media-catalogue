//! TV channels: three schedules over the films and shows, worked out like the
//! radio's stations. Nothing runs in the background: what is on at any moment
//! is arithmetic from a fixed start, so switching on joins a film part way
//! through, as on a real television.
//!
//!   1 Films       every film, shuffled, a new order every time round
//!   2 Series      every show, one episode each in turn, in order
//!   3 Everything  films and episodes shuffled together, a show kept apart
//!                 from itself

use crate::stations::{self, Entry, EPOCH_MS};
use crate::video;
use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;

pub const NAMES: [&str; 3] = ["Films", "Series", "Everything"];

/// What a slot holds: a film or an episode, by id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "lowercase")]
pub enum Item {
    Movie(i64),
    Episode(i64),
}

impl Item {
    fn code(self) -> i64 {
        match self {
            Item::Movie(id) => id * 2,
            Item::Episode(id) => id * 2 + 1,
        }
    }
    fn from_code(c: i64) -> Self {
        if c % 2 == 0 {
            Item::Movie(c / 2)
        } else {
            Item::Episode(c / 2)
        }
    }
}

/// A turn on a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Slot {
    pub item: Item,
    pub start_ms: i64,
    pub duration_ms: i64,
}

impl Slot {
    pub fn end_ms(&self) -> i64 {
        self.start_ms + self.duration_ms
    }
}

fn movies(conn: &Connection) -> Result<Vec<Entry>> {
    let mut st =
        conn.prepare("SELECT id, duration_ms FROM movies WHERE duration_ms > 0 ORDER BY id")?;
    let rows = st.query_map([], |r| {
        let id: i64 = r.get(0)?;
        Ok(Entry {
            track_id: Item::Movie(id).code(),
            // Each film its own "artist": nothing to keep apart.
            artist_id: -id,
            duration_ms: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Episodes with their show and their place in it, in order.
fn episodes(conn: &Connection) -> Result<Vec<(i64, Entry)>> {
    let mut st = conn.prepare(
        "SELECT e.id, e.show_id, e.duration_ms FROM episodes e JOIN shows s ON s.id = e.show_id
         WHERE e.duration_ms > 0 ORDER BY s.title COLLATE NOCASE, e.show_id, e.season, e.episode, e.path",
    )?;
    let rows = st.query_map(params![], |r| {
        let show: i64 = r.get(1)?;
        Ok((
            show,
            Entry {
                track_id: Item::Episode(r.get(0)?).code(),
                artist_id: show,
                duration_ms: r.get(2)?,
            },
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Shows taken in turn, one episode each, until every episode has aired:
/// A1 B1 C1 A2 B2 C2 A3 C3 ...
fn round_robin(eps: Vec<(i64, Entry)>) -> Vec<Entry> {
    let mut shows: Vec<(i64, std::collections::VecDeque<Entry>)> = Vec::new();
    for (show, e) in eps {
        match shows.iter_mut().find(|(s, _)| *s == show) {
            Some((_, q)) => q.push_back(e),
            None => shows.push((show, [e].into())),
        }
    }
    let mut out = Vec::new();
    loop {
        let mut any = false;
        for (_, q) in shows.iter_mut() {
            if let Some(e) = q.pop_front() {
                out.push(e);
                any = true;
            }
        }
        if !any {
            return out;
        }
    }
}

/// The entries of channel `n` (0, 1 or 2).
pub fn entries(conn: &Connection, n: usize) -> Result<Vec<Entry>> {
    Ok(match n {
        0 => movies(conn)?,
        1 => round_robin(episodes(conn)?),
        _ => {
            let mut all = movies(conn)?;
            all.extend(episodes(conn)?.into_iter().map(|(_, e)| e));
            all
        }
    })
}

/// What channel `n` has on at `at_ms`.
pub fn slot_at(entries: &[Entry], n: usize, at_ms: i64) -> Option<Slot> {
    let s = if n == 1 {
        in_order(entries, at_ms)?
    } else {
        // Keys apart from every playlist's, so a channel never mirrors a station.
        stations::slot_at(entries, -1 - n as i64, at_ms)?
    };
    Some(Slot {
        item: Item::from_code(s.track_id),
        start_ms: s.start_ms,
        duration_ms: s.duration_ms,
    })
}

/// The same order every time round.
fn in_order(entries: &[Entry], at_ms: i64) -> Option<stations::Slot> {
    let total: i64 = entries.iter().map(|e| e.duration_ms).sum();
    if total <= 0 {
        return None;
    }
    let since = at_ms - EPOCH_MS;
    let mut start = EPOCH_MS + since.div_euclid(total) * total;
    let within = since.rem_euclid(total);
    let mut cum = 0;
    for e in entries {
        if within < cum + e.duration_ms {
            return Some(stations::Slot {
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

/// A channel as the window lists it.
#[derive(Debug, Clone, Serialize)]
pub struct Channel {
    /// 1, 2 or 3, as on the set.
    pub number: usize,
    pub name: String,
    pub items: usize,
    pub now: Option<Showing>,
    pub next: Option<Showing>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Showing {
    #[serde(flatten)]
    pub item: Item,
    pub title: String,
    /// For an episode: "S01E03 · Zen".
    pub subtitle: Option<String>,
    pub year: Option<i64>,
    pub poster_id: Option<i64>,
    pub start_ms: i64,
    pub position_ms: i64,
    pub duration_ms: i64,
}

fn describe(conn: &Connection, s: Slot, at_ms: i64) -> Result<Option<Showing>> {
    let base = |title: String, subtitle, year, poster_id| Showing {
        item: s.item,
        title,
        subtitle,
        year,
        poster_id,
        start_ms: s.start_ms,
        position_ms: (at_ms - s.start_ms).clamp(0, s.duration_ms),
        duration_ms: s.duration_ms,
    };
    Ok(match s.item {
        Item::Movie(id) => {
            video::movie(conn, id)?.map(|m| base(m.title, None, m.year, m.poster_id))
        }
        Item::Episode(id) => {
            let Some(e) = video::episode(conn, id)? else {
                return Ok(None);
            };
            let show = video::show(conn, e.show_id)?;
            let mut sub = format!("S{:02}E{:02}", e.season, e.episode);
            if let Some(t) = e.title {
                sub.push_str(&format!(" · {t}"));
            }
            Some(match show {
                Some(sh) => base(sh.title, Some(sub), sh.year, sh.poster_id),
                None => base(String::new(), Some(sub), None, None),
            })
        }
    })
}

/// The three channels with what is on at `at_ms` and what follows.
pub fn listing(conn: &Connection, at_ms: i64) -> Result<Vec<Channel>> {
    let mut out = Vec::new();
    for (n, name) in NAMES.iter().enumerate() {
        let e = entries(conn, n)?;
        let now = slot_at(&e, n, at_ms);
        let next = now.and_then(|s| slot_at(&e, n, s.end_ms()));
        out.push(Channel {
            number: n + 1,
            name: name.to_string(),
            items: e.len(),
            now: now.map(|s| describe(conn, s, at_ms)).transpose()?.flatten(),
            next: next
                .map(|s| describe(conn, s, at_ms))
                .transpose()?
                .flatten(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(id: i64, show: i64) -> (i64, Entry) {
        (
            show,
            Entry {
                track_id: Item::Episode(id).code(),
                artist_id: show,
                duration_ms: 1_000,
            },
        )
    }

    #[test]
    fn shows_take_turns_in_order() {
        let order: Vec<i64> = round_robin(vec![
            ep(1, 10),
            ep(2, 10),
            ep(3, 10),
            ep(4, 20),
            ep(5, 30),
            ep(6, 30),
        ])
        .into_iter()
        .map(|e| e.track_id)
        .collect();
        let want: Vec<i64> = [1, 4, 5, 2, 6, 3]
            .iter()
            .map(|&i| Item::Episode(i).code())
            .collect();
        assert_eq!(order, want);
    }

    #[test]
    fn items_round_trip_and_the_series_channel_follows_its_order() {
        assert_eq!(Item::from_code(Item::Movie(7).code()), Item::Movie(7));
        assert_eq!(Item::from_code(Item::Episode(7).code()), Item::Episode(7));
        let e = round_robin(vec![ep(1, 10), ep(2, 10), ep(3, 20)]);
        let s = slot_at(&e, 1, EPOCH_MS + 1_500).unwrap();
        assert_eq!(s.item, Item::Episode(3));
        assert_eq!(s.start_ms, EPOCH_MS + 1_000);
        // Round again after the last.
        assert_eq!(
            slot_at(&e, 1, EPOCH_MS + 3_000).unwrap().item,
            Item::Episode(1)
        );
        assert_eq!(slot_at(&[], 0, EPOCH_MS), None);
    }

    #[test]
    fn channels_list_what_is_on() {
        let path = std::env::temp_dir().join(format!("katalog-channels-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let conn = crate::db::open(&path).unwrap();
        conn.execute_batch(
            "INSERT INTO movies (id, title, year, path, size, mtime, duration_ms) VALUES
               (1, 'Stalker', 1979, '/m1', 0, 0, 9600000), (2, 'Alien', 1979, '/m2', 0, 0, 7000000);
             INSERT INTO shows (id, title, dir) VALUES (1, 'Twin Peaks', '/s1');
             INSERT INTO episodes (id, show_id, season, episode, title, path, size, mtime, duration_ms) VALUES
               (1, 1, 1, 1, 'Pilot', '/e1', 0, 0, 5400000), (2, 1, 1, 2, NULL, '/e2', 0, 0, 2800000);",
        )
        .unwrap();
        let l = listing(&conn, EPOCH_MS + 60_000).unwrap();
        assert_eq!(l.iter().map(|c| c.items).collect::<Vec<_>>(), [2, 2, 4]);
        let series = l[1].now.as_ref().unwrap();
        assert_eq!(
            (series.title.as_str(), series.subtitle.as_deref()),
            ("Twin Peaks", Some("S01E01 · Pilot"))
        );
        assert_eq!(series.position_ms, 60_000);
        assert_eq!(
            l[1].next.as_ref().unwrap().subtitle.as_deref(),
            Some("S01E02")
        );
        assert!(l[0].now.is_some() && l[2].now.is_some());
        let _ = std::fs::remove_file(path);
    }
}
