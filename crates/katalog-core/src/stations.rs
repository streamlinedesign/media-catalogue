//! Stations: the radio. A station is a row of its own, separate from the
//! playlists: the whole library, one playlist, or one genre. Nothing runs in
//! the background. A station is a schedule: its tracks in a shuffled order
//! that changes every time round, laid end to end from a fixed start in 2026.
//! What is on at any moment is arithmetic, so tuning in joins a song part way
//! through, like a real radio, and costs nothing until then.
//!
//! The order keeps the same artist from playing twice in a row where the
//! station allows it. Adding to or removing from a playlist changes its
//! schedule from then on. Each station keeps its own seed, so renaming it or
//! moving it on the dial does not change what it plays.

use crate::db;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

/// Where every schedule starts: 2026-01-01T00:00:00Z, in Unix milliseconds.
pub const EPOCH_MS: i64 = 1_767_225_600_000;

const SCHEMA: &str = r#"
CREATE TABLE stations (
  id          INTEGER PRIMARY KEY,
  name        TEXT NOT NULL UNIQUE,
  kind        TEXT NOT NULL,             -- library | playlist | genre
  playlist_id INTEGER REFERENCES playlists(id) ON DELETE CASCADE,
  genre       TEXT,                      -- lower case, as split_genres gives it
  seed        INTEGER NOT NULL,          -- tells the shuffles apart; never changes
  position    INTEGER,                   -- place on the dial
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
"#;

/// Makes the stations table the first time a database meets it: the whole
/// library first, then every playlist that was on the dial, each seeded with
/// its playlist id as before so what is on air does not change. A genre
/// playlist on the dial becomes a genre station, which a rescan cannot take
/// away. `playlists.station` stays in old databases but is no longer read.
pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'stations')",
        [],
        |r| r.get(0),
    )?;
    if exists {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(SCHEMA)?;
    tx.execute(
        "INSERT INTO stations (name, kind, seed, position) VALUES ('Everything', 'library', 0, 0)",
        [],
    )?;
    let old: Vec<(i64, String, Option<String>)> = tx
        .prepare(
            "SELECT id, name, genre FROM playlists WHERE station <> 0 ORDER BY name COLLATE NOCASE",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (i, (id, name, genre)) in old.into_iter().enumerate() {
        let name = if name_taken(&tx, &name)? {
            format!("{name} (playlist)")
        } else {
            name
        };
        let (kind, playlist) = match genre {
            Some(_) => (Kind::Genre, None),
            None => (Kind::Playlist, Some(id)),
        };
        tx.execute(
            "INSERT INTO stations (name, kind, playlist_id, genre, seed, position)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![name, kind.as_str(), playlist, genre, id, i as i64 + 1],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// What a station plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Every track in the library.
    Library,
    /// The tracks of one playlist.
    Playlist,
    /// Every track whose genre tag names this genre.
    Genre,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Library => "library",
            Kind::Playlist => "playlist",
            Kind::Genre => "genre",
        }
    }

    fn parse(s: &str) -> Kind {
        match s {
            "playlist" => Kind::Playlist,
            "genre" => Kind::Genre,
            _ => Kind::Library,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Station {
    pub id: i64,
    pub name: String,
    pub kind: Kind,
    /// For a playlist station.
    pub playlist: Option<i64>,
    /// For a genre station: the genre in lower case.
    pub genre: Option<String>,
    /// The schedule's key. Stays put when the station is renamed or moved.
    pub seed: i64,
    pub position: i64,
}

/// A station to make. Without a name it takes the playlist's or the genre's.
#[derive(Debug, Clone, Deserialize)]
pub struct NewStation {
    #[serde(default)]
    pub name: String,
    pub kind: Kind,
    #[serde(default)]
    pub playlist: Option<i64>,
    #[serde(default)]
    pub genre: Option<String>,
}

const STATION_SQL: &str =
    "SELECT id, name, kind, playlist_id, genre, seed, COALESCE(position, id) FROM stations";

fn station_from_row(r: &Row) -> rusqlite::Result<Station> {
    Ok(Station {
        id: r.get(0)?,
        name: r.get(1)?,
        kind: Kind::parse(&r.get::<_, String>(2)?),
        playlist: r.get(3)?,
        genre: r.get(4)?,
        seed: r.get(5)?,
        position: r.get(6)?,
    })
}

fn name_taken(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM stations WHERE name = ?1 COLLATE NOCASE)",
        [name],
        |r| r.get(0),
    )?)
}

/// The key a genre station keeps: the first genre in `g`, in lower case, as
/// `db::split_genres` would find it in a tag.
pub fn genre_key(g: &str) -> Option<String> {
    db::split_genres(g).first().map(|g| g.to_lowercase())
}

/// Every station in dial order.
pub fn list(conn: &Connection) -> Result<Vec<Station>> {
    let mut st = conn.prepare(&format!(
        "{STATION_SQL} ORDER BY COALESCE(position, id), id"
    ))?;
    let rows = st.query_map([], station_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<Station>> {
    Ok(conn
        .query_row(
            &format!("{STATION_SQL} WHERE id = ?1"),
            [id],
            station_from_row,
        )
        .optional()?)
}

/// A station by its name, case-insensitively.
pub fn by_name(conn: &Connection, name: &str) -> Result<Option<Station>> {
    Ok(conn
        .query_row(
            &format!("{STATION_SQL} WHERE name = ?1 COLLATE NOCASE"),
            [name.trim()],
            station_from_row,
        )
        .optional()?)
}

/// The first whole-library station on the dial: what "the radio" with no
/// station named means.
pub fn library(conn: &Connection) -> Result<Option<Station>> {
    Ok(conn
        .query_row(
            &format!(
                "{STATION_SQL} WHERE kind = 'library' ORDER BY COALESCE(position, id), id LIMIT 1"
            ),
            [],
            station_from_row,
        )
        .optional()?)
}

/// The station that plays a playlist: its playlist station, or for a genre
/// playlist, the station of that genre.
pub fn of_playlist(conn: &Connection, playlist: i64) -> Result<Option<Station>> {
    Ok(conn
        .query_row(
            &format!(
                "{STATION_SQL} WHERE playlist_id = ?1
                   OR (kind = 'genre' AND genre = (SELECT genre FROM playlists WHERE id = ?1))
                 ORDER BY kind = 'playlist' DESC, COALESCE(position, id) LIMIT 1"
            ),
            [playlist],
            station_from_row,
        )
        .optional()?)
}

/// A station a name in a URL or on the command line means: a station id or
/// name, "everything" or "library", or for older links a playlist id or name.
pub fn resolve(conn: &Connection, sel: &str) -> Result<Option<Station>> {
    let sel = sel.trim();
    if let Ok(id) = sel.parse::<i64>() {
        if let Some(s) = get(conn, id)? {
            return Ok(Some(s));
        }
    }
    if let Some(s) = by_name(conn, sel)? {
        return Ok(Some(s));
    }
    if sel.eq_ignore_ascii_case("everything") || sel.eq_ignore_ascii_case("library") {
        return library(conn);
    }
    let playlist = match sel.parse::<i64>() {
        Ok(id) => db::playlist(conn, id)?,
        Err(_) => db::playlist_by_name(conn, sel)?,
    };
    match playlist {
        Some(p) => of_playlist(conn, p.id),
        None => Ok(None),
    }
}

pub fn create(conn: &Connection, new: &NewStation) -> Result<Station> {
    let (playlist, genre, default_name) = match new.kind {
        Kind::Library => (None, None, "Everything".to_string()),
        Kind::Playlist => {
            let id = new
                .playlist
                .ok_or_else(|| anyhow::anyhow!("a playlist station needs a playlist"))?;
            let p = db::playlist(conn, id)?
                .ok_or_else(|| anyhow::anyhow!("there is no playlist {id}"))?;
            let taken: bool = conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM stations WHERE playlist_id = ?1)",
                [id],
                |r| r.get(0),
            )?;
            if taken {
                anyhow::bail!("{} has a station already", p.name);
            }
            (Some(id), None, p.name)
        }
        Kind::Genre => {
            let given = new.genre.as_deref().unwrap_or("");
            let key =
                genre_key(given).ok_or_else(|| anyhow::anyhow!("a genre station needs a genre"))?;
            let taken: bool = conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM stations WHERE kind = 'genre' AND genre = ?1)",
                [&key],
                |r| r.get(0),
            )?;
            if taken {
                anyhow::bail!("{key} has a station already");
            }
            let name = db::split_genres(given).remove(0);
            (None, Some(key), name)
        }
    };
    let name = match new.name.trim() {
        "" => default_name,
        n => n.to_string(),
    };
    if name_taken(conn, &name)? {
        anyhow::bail!("there is already a station called {name}");
    }
    // A playlist's station keeps the playlist id as its seed when it can,
    // so taking it off the dial and back on plays the same schedule.
    let free = |seed: i64| -> Result<bool> {
        Ok(!conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM stations WHERE seed = ?1)",
            [seed],
            |r| r.get::<_, bool>(0),
        )?)
    };
    let seed = match playlist {
        Some(p) if free(p)? => p,
        _ => conn.query_row(
            "SELECT MAX(COALESCE((SELECT MAX(seed) FROM stations), 0),
                        COALESCE((SELECT MAX(id) FROM playlists), 0)) + 1",
            [],
            |r| r.get(0),
        )?,
    };
    conn.execute(
        "INSERT INTO stations (name, kind, playlist_id, genre, seed, position)
         VALUES (?1, ?2, ?3, ?4, ?5, (SELECT COALESCE(MAX(position), -1) + 1 FROM stations))",
        params![name, new.kind.as_str(), playlist, genre, seed],
    )?;
    Ok(get(conn, conn.last_insert_rowid())?.expect("just inserted"))
}

/// Renames a station. False when there is no such station.
pub fn rename(conn: &Connection, id: i64, name: &str) -> Result<bool> {
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("a station needs a name");
    }
    if let Some(other) = by_name(conn, name)? {
        if other.id != id {
            anyhow::bail!("there is already a station called {name}");
        }
    }
    Ok(conn.execute(
        "UPDATE stations SET name = ?1 WHERE id = ?2",
        params![name, id],
    )? > 0)
}

/// Takes a station off the dial. Its playlist, if any, stays.
pub fn delete(conn: &Connection, id: i64) -> Result<bool> {
    Ok(conn.execute("DELETE FROM stations WHERE id = ?1", [id])? > 0)
}

/// Puts the stations named in `ids` first on the dial, in that order; the
/// rest keep their order after them.
pub fn reorder(conn: &Connection, ids: &[i64]) -> Result<()> {
    let mut order: Vec<i64> = ids.to_vec();
    for s in list(conn)? {
        if !order.contains(&s.id) {
            order.push(s.id);
        }
    }
    for (i, id) in order.iter().enumerate() {
        conn.execute(
            "UPDATE stations SET position = ?1 WHERE id = ?2",
            params![i as i64, id],
        )?;
    }
    Ok(())
}

/// Puts a playlist on the dial (makes its station, a genre station for a
/// genre playlist) or takes it off (deletes whatever station plays it).
pub fn set_for_playlist(conn: &Connection, playlist: i64, on: bool) -> Result<()> {
    if !on {
        while let Some(s) = of_playlist(conn, playlist)? {
            delete(conn, s.id)?;
        }
        return Ok(());
    }
    if of_playlist(conn, playlist)?.is_some() {
        return Ok(());
    }
    let p = db::playlist(conn, playlist)?
        .ok_or_else(|| anyhow::anyhow!("there is no playlist {playlist}"))?;
    let name = if name_taken(conn, &p.name)? {
        format!("{} (playlist)", p.name)
    } else {
        p.name.clone()
    };
    let new = match &p.genre {
        Some(g) => NewStation {
            name,
            kind: Kind::Genre,
            playlist: None,
            genre: Some(g.clone()),
        },
        None => NewStation {
            name,
            kind: Kind::Playlist,
            playlist: Some(playlist),
            genre: None,
        },
    };
    create(conn, &new)?;
    Ok(())
}

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

fn entry_from_row(r: &Row) -> rusqlite::Result<Entry> {
    Ok(Entry {
        track_id: r.get(0)?,
        artist_id: r.get(1)?,
        duration_ms: r.get(2)?,
    })
}

/// Every track with a genre tag and a length, with the tag: what genre
/// stations pick from. Read once per listing, however many genre stations.
type Tagged = Vec<(Entry, String)>;

fn tagged(conn: &Connection) -> Result<Tagged> {
    let mut st = conn.prepare(
        "SELECT id, artist_id, duration_ms, genre FROM tracks
         WHERE duration_ms > 0 AND genre IS NOT NULL AND genre <> '' ORDER BY id",
    )?;
    let rows = st.query_map([], |r| Ok((entry_from_row(r)?, r.get(3)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn of_genre(tagged: &Tagged, key: &str) -> Vec<Entry> {
    tagged
        .iter()
        .filter(|(_, tag)| {
            db::split_genres(tag)
                .iter()
                .any(|g| g.to_lowercase() == key)
        })
        .map(|(e, _)| e.clone())
        .collect()
}

/// The tracks of a station, in track id order. Tracks without a known
/// length cannot be scheduled and are left out.
pub fn entries(conn: &Connection, station: &Station) -> Result<Vec<Entry>> {
    entries_with(conn, station, &mut None)
}

fn entries_with(
    conn: &Connection,
    station: &Station,
    cache: &mut Option<Tagged>,
) -> Result<Vec<Entry>> {
    match station.kind {
        Kind::Library => {
            let mut st = conn.prepare(
                "SELECT id, artist_id, duration_ms FROM tracks WHERE duration_ms > 0 ORDER BY id",
            )?;
            let rows = st.query_map([], entry_from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        }
        Kind::Playlist => {
            let mut st = conn.prepare(
                "SELECT t.id, t.artist_id, t.duration_ms FROM tracks t
                 WHERE t.duration_ms > 0
                   AND t.id IN (SELECT track_id FROM playlist_tracks WHERE playlist_id = ?1)
                 ORDER BY t.id",
            )?;
            let rows = st.query_map(params![station.playlist], entry_from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        }
        Kind::Genre => {
            let Some(key) = &station.genre else {
                return Ok(Vec::new());
            };
            if cache.is_none() {
                *cache = Some(tagged(conn)?);
            }
            Ok(of_genre(cache.as_ref().expect("just read"), key))
        }
    }
}

/// The slot playing on a station at `at_ms`. `key` tells stations apart (the
/// station's seed) so two stations with the same tracks still play them in
/// different orders.
pub fn slot_at(entries: &[Entry], key: i64, at_ms: i64) -> Option<Slot> {
    slots_from(entries, key, at_ms, 1).into_iter().next()
}

/// The slot playing at `at_ms` and the `n - 1` after it, working out each
/// time round's order once.
pub fn slots_from(entries: &[Entry], key: i64, at_ms: i64, n: usize) -> Vec<Slot> {
    let total: i64 = entries.iter().map(|e| e.duration_ms).sum();
    let mut out = Vec::with_capacity(n);
    if total <= 0 || n == 0 {
        return out;
    }
    let since = at_ms - EPOCH_MS;
    let mut cycle = since.div_euclid(total);
    let within = since.rem_euclid(total);
    let mut start = EPOCH_MS + cycle * total;
    let mut cum = 0;
    loop {
        for i in order(entries, key, cycle) {
            let e = &entries[i];
            // Skip what ended before `at_ms` in the first time round.
            if out.is_empty() && within >= cum + e.duration_ms {
                cum += e.duration_ms;
                start += e.duration_ms;
                continue;
            }
            out.push(Slot {
                track_id: e.track_id,
                start_ms: start,
                duration_ms: e.duration_ms,
            });
            start += e.duration_ms;
            if out.len() == n {
                return out;
            }
        }
        cycle += 1;
    }
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
    /// The station's id.
    pub id: i64,
    pub kind: Kind,
    /// The playlist for a playlist station, None otherwise.
    pub playlist: Option<i64>,
    /// The genre for a genre station, in lower case.
    pub genre: Option<String>,
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

/// Every station in dial order, with what is on at `at_ms`.
pub fn on_air(conn: &Connection, at_ms: i64) -> Result<Vec<OnAir>> {
    let mut cache = None;
    list(conn)?
        .iter()
        .map(|s| on_air_with(conn, s, at_ms, &mut cache))
        .collect()
}

/// One station with what is on at `at_ms`.
pub fn on_air_one(conn: &Connection, station: &Station, at_ms: i64) -> Result<OnAir> {
    on_air_with(conn, station, at_ms, &mut None)
}

fn on_air_with(
    conn: &Connection,
    station: &Station,
    at_ms: i64,
    cache: &mut Option<Tagged>,
) -> Result<OnAir> {
    let e = entries_with(conn, station, cache)?;
    let describe = |slot: Option<&Slot>, position_ms: i64| -> Result<Option<Playing>> {
        let Some(s) = slot else { return Ok(None) };
        Ok(db::track(conn, s.track_id)?.map(|t| Playing {
            track_id: t.id,
            title: t.title,
            artist: t.artist,
            position_ms,
            duration_ms: s.duration_ms,
        }))
    };
    let slots = slots_from(&e, station.seed, at_ms, 2);
    let now = slots.first();
    Ok(OnAir {
        id: station.id,
        kind: station.kind,
        playlist: station.playlist,
        genre: station.genre.clone(),
        name: station.name.clone(),
        tracks: e.len(),
        now: describe(now, now.map_or(0, |s| (at_ms - s.start_ms).max(0)))?,
        next: describe(slots.get(1), 0)?,
    })
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

    #[test]
    fn slots_from_matches_slot_at_across_rounds() {
        let e = station(9, 3);
        let total: i64 = e.iter().map(|e| e.duration_ms).sum();
        // Start near the end of a round so the run crosses into the next.
        let mut at = EPOCH_MS + 5 * total - 1000;
        let run = slots_from(&e, 4, at, 12);
        assert_eq!(run.len(), 12);
        for s in run {
            assert_eq!(slot_at(&e, 4, at), Some(s));
            at = s.end_ms();
        }
    }

    /// A database file of its own under the temp dir, never the real one.
    fn temp_db(tag: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("katalog-stations-{tag}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    fn add_tracks(conn: &Connection, genres: &[&str]) {
        conn.execute_batch(
            "INSERT OR IGNORE INTO artists (id, name, sort_name) VALUES (1, 'Azra', 'Azra'), (2, 'Haustor', 'Haustor');
             INSERT OR IGNORE INTO albums (id, artist_id, title, dir) VALUES (1, 1, 'A', '/a');",
        )
        .unwrap();
        for (n, g) in genres.iter().enumerate() {
            conn.execute(
                "INSERT INTO tracks (album_id, artist_id, title, path, size, mtime, codec, genre, duration_ms)
                 VALUES (1, ?1, ?2, ?3, 1, 1, 'flac', ?4, ?5)",
                params![1 + n as i64 % 2, format!("t{n}"), format!("/a/{n}-{g}.flac"), g, 180_000 + n as i64 * 1000],
            )
            .unwrap();
        }
    }

    #[test]
    fn old_dial_playlists_become_stations_with_their_ids_as_seeds() {
        let path = temp_db("migrate");
        {
            // A database as 0.18 left it: no stations table, the dial in
            // playlists.station.
            let old = Connection::open(&path).unwrap();
            old.execute_batch(
                "CREATE TABLE playlists (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE,
                   created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
                   station INTEGER NOT NULL DEFAULT 1, genre TEXT);
                 INSERT INTO playlists (id, name, station) VALUES (4, 'Chill', 1), (7, 'Off', 0), (9, 'Bus', 1);
                 INSERT INTO playlists (id, name, station, genre) VALUES (12, 'New Wave', 1, 'new wave');",
            )
            .unwrap();
        }
        let conn = db::open(&path).unwrap();
        let all = list(&conn).unwrap();
        let got: Vec<(&str, Kind, Option<i64>, Option<&str>, i64)> = all
            .iter()
            .map(|s| {
                (
                    s.name.as_str(),
                    s.kind,
                    s.playlist,
                    s.genre.as_deref(),
                    s.seed,
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                ("Everything", Kind::Library, None, None, 0),
                ("Bus", Kind::Playlist, Some(9), None, 9),
                ("Chill", Kind::Playlist, Some(4), None, 4),
                ("New Wave", Kind::Genre, None, Some("new wave"), 12),
            ]
        );
        // Opening again changes nothing.
        drop(conn);
        let conn = db::open(&path).unwrap();
        assert_eq!(list(&conn).unwrap(), all);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_genre_station_outlives_its_genre_playlist() {
        let path = temp_db("genre");
        let conn = db::open(&path).unwrap();
        let mut tags = vec!["Rock; New Wave"; 9];
        tags.push("Jazz");
        add_tracks(&conn, &tags);
        db::sync_genre_playlists(&conn).unwrap();
        let wave = db::playlists(&conn)
            .unwrap()
            .into_iter()
            .find(|p| p.genre.as_deref() == Some("new wave"))
            .unwrap();
        db::set_playlist_station(&conn, wave.id, true).unwrap();
        let st = of_playlist(&conn, wave.id).unwrap().unwrap();
        assert_eq!(st.kind, Kind::Genre);
        assert_eq!(entries(&conn, &st).unwrap().len(), 9);
        // Too few tracks left for a genre playlist: it goes, the station stays
        // and plays what the genre has.
        conn.execute("UPDATE tracks SET genre = 'Jazz' WHERE id > 3", [])
            .unwrap();
        db::sync_genre_playlists(&conn).unwrap();
        assert!(db::playlist(&conn, wave.id).unwrap().is_none());
        let st = get(&conn, st.id).unwrap().unwrap();
        assert_eq!(entries(&conn, &st).unwrap().len(), 3);
        // A playlist station goes with its playlist.
        let mine = db::create_playlist(&conn, "Mine").unwrap();
        db::add_to_playlist(&conn, mine.id, &[1, 2]).unwrap();
        let made = create(
            &conn,
            &NewStation {
                name: String::new(),
                kind: Kind::Playlist,
                playlist: Some(mine.id),
                genre: None,
            },
        )
        .unwrap();
        assert_eq!((made.name.as_str(), made.seed), ("Mine", mine.id));
        db::delete_playlist(&conn, mine.id).unwrap();
        assert!(get(&conn, made.id).unwrap().is_none());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn on_air_follows_the_dial_and_each_seed() {
        let path = temp_db("onair");
        let conn = db::open(&path).unwrap();
        add_tracks(&conn, &["Rock"; 6]);
        let jazz = create(
            &conn,
            &NewStation {
                name: "Late".into(),
                kind: Kind::Genre,
                playlist: None,
                genre: Some("Rock".into()),
            },
        )
        .unwrap();
        assert!(create(
            &conn,
            &NewStation {
                name: "late".into(),
                kind: Kind::Library,
                playlist: None,
                genre: None,
            },
        )
        .is_err());
        let at = EPOCH_MS + 987_654_321;
        let names = |conn: &Connection| -> Vec<String> {
            on_air(conn, at)
                .unwrap()
                .into_iter()
                .map(|a| a.name)
                .collect()
        };
        assert_eq!(names(&conn), ["Everything", "Late"]);
        reorder(&conn, &[jazz.id]).unwrap();
        assert_eq!(names(&conn), ["Late", "Everything"]);
        rename(&conn, jazz.id, "Rock all night").unwrap();
        let air = on_air(&conn, at).unwrap();
        assert_eq!(air[0].name, "Rock all night");
        assert_eq!(air[0].id, jazz.id);
        assert_eq!(air[0].kind, Kind::Genre);
        assert_eq!(air[0].playlist, None);
        assert_eq!(air[0].tracks, 6);
        // What is on comes from the station's own seed and says how far in.
        let e = entries(&conn, &jazz).unwrap();
        let now = slot_at(&e, jazz.seed, at).unwrap();
        let on = air[0].now.as_ref().unwrap();
        assert_eq!(on.track_id, now.track_id);
        assert_eq!(on.position_ms, at - now.start_ms);
        let next = slot_at(&e, jazz.seed, now.end_ms()).unwrap();
        assert_eq!(air[0].next.as_ref().unwrap().track_id, next.track_id);
        assert_eq!(air[0].next.as_ref().unwrap().position_ms, 0);
        assert!(delete(&conn, jazz.id).unwrap());
        assert_eq!(names(&conn), ["Everything"]);
        let _ = std::fs::remove_file(path);
    }
}
