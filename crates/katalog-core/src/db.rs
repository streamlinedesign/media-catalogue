//! SQLite storage. One file holds the whole library.
//!
//! The schema is plain SQL so it can move to Postgres later without a rewrite.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use std::path::Path;

pub const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS artists (
  id        INTEGER PRIMARY KEY,
  name      TEXT NOT NULL UNIQUE,
  sort_name TEXT NOT NULL,
  mbid      TEXT
);

CREATE TABLE IF NOT EXISTS albums (
  id         INTEGER PRIMARY KEY,
  artist_id  INTEGER NOT NULL REFERENCES artists(id),
  title      TEXT NOT NULL,
  year       INTEGER,
  dir        TEXT NOT NULL,             -- folder the album lives in
  mbid       TEXT,
  art_id     INTEGER REFERENCES art(id),
  art_tried  INTEGER NOT NULL DEFAULT 0, -- 1 once an online lookup found nothing
  added_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE (artist_id, title, dir)
);
CREATE INDEX IF NOT EXISTS albums_title ON albums(title);

CREATE TABLE IF NOT EXISTS tracks (
  id           INTEGER PRIMARY KEY,
  album_id     INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
  artist_id    INTEGER NOT NULL REFERENCES artists(id),
  title        TEXT NOT NULL,
  disc         INTEGER,
  number       INTEGER,
  duration_ms  INTEGER NOT NULL DEFAULT 0,
  path         TEXT NOT NULL UNIQUE,
  size         INTEGER NOT NULL,
  mtime        INTEGER NOT NULL,        -- unix seconds, to skip unchanged files
  codec        TEXT NOT NULL,
  sample_rate  INTEGER,
  bit_depth    INTEGER,
  channels     INTEGER,
  genre        TEXT,
  lyrics_tried INTEGER NOT NULL DEFAULT 0,
  added_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX IF NOT EXISTS tracks_album ON tracks(album_id, disc, number);
CREATE INDEX IF NOT EXISTS tracks_title ON tracks(title);

CREATE TABLE IF NOT EXISTS art (
  id      INTEGER PRIMARY KEY,
  hash    TEXT NOT NULL UNIQUE,         -- blake3 of the bytes, so one image is stored once
  mime    TEXT NOT NULL,
  data    BLOB NOT NULL,
  source  TEXT NOT NULL                 -- embedded | folder | coverartarchive
);

CREATE TABLE IF NOT EXISTS art_thumbs (
  art_id  INTEGER NOT NULL REFERENCES art(id) ON DELETE CASCADE,
  size    INTEGER NOT NULL,               -- longest side in pixels
  data    BLOB NOT NULL,                  -- JPEG
  PRIMARY KEY (art_id, size)
);

CREATE TABLE IF NOT EXISTS lyrics (
  track_id  INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
  synced    INTEGER NOT NULL,           -- 1 when the text carries [mm:ss.xx] timestamps
  text      TEXT NOT NULL,
  source    TEXT NOT NULL               -- embedded | lrc | lrclib
);

-- Everything below is for later milestones, created now so the schema is settled.
CREATE TABLE IF NOT EXISTS plays (
  id          INTEGER PRIMARY KEY,
  track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
  started_at  TEXT NOT NULL,
  finished    INTEGER NOT NULL DEFAULT 0,
  source      TEXT NOT NULL DEFAULT 'manual'   -- manual | radio | playlist
);
CREATE INDEX IF NOT EXISTS plays_track ON plays(track_id, started_at);

CREATE TABLE IF NOT EXISTS track_tags (
  track_id  INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
  tag       TEXT NOT NULL,              -- your own words: calm, heavy, bad-mood ...
  PRIMARY KEY (track_id, tag)
);

CREATE TABLE IF NOT EXISTS playlists (
  id         INTEGER PRIMARY KEY,
  name       TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE TABLE IF NOT EXISTS playlist_tracks (
  playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
  position    INTEGER NOT NULL,
  track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
  PRIMARY KEY (playlist_id, position)
);

CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
"#;

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    // WAL lets the player read while the scanner writes.
    let _: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

#[derive(Debug, Clone, Serialize)]
pub struct Artist {
    pub id: i64,
    pub name: String,
    pub album_count: i64,
    pub track_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Album {
    pub id: i64,
    pub title: String,
    pub artist_id: i64,
    pub artist: String,
    pub year: Option<i64>,
    pub art_id: Option<i64>,
    pub track_count: i64,
    pub duration_ms: i64,
    pub dir: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Track {
    pub id: i64,
    pub title: String,
    pub album_id: i64,
    pub album: String,
    pub artist_id: i64,
    pub artist: String,
    pub album_artist: String,
    pub disc: Option<i64>,
    pub number: Option<i64>,
    pub duration_ms: i64,
    pub path: String,
    pub codec: String,
    pub sample_rate: Option<i64>,
    pub bit_depth: Option<i64>,
    pub channels: Option<i64>,
    pub genre: Option<String>,
    pub year: Option<i64>,
    pub art_id: Option<i64>,
    pub has_lyrics: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Lyrics {
    pub track_id: i64,
    pub synced: bool,
    pub text: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub artists: i64,
    pub albums: i64,
    pub tracks: i64,
    pub duration_ms: i64,
    pub bytes: i64,
    pub albums_without_art: i64,
    pub tracks_without_lyrics: i64,
    pub last_scan: Option<String>,
}

const ALBUM_SQL: &str = r#"
SELECT al.id, al.title, al.artist_id, ar.name, al.year, al.art_id,
       (SELECT COUNT(*) FROM tracks t WHERE t.album_id = al.id),
       (SELECT COALESCE(SUM(duration_ms), 0) FROM tracks t WHERE t.album_id = al.id),
       al.dir
FROM albums al JOIN artists ar ON ar.id = al.artist_id
"#;

fn album_from_row(r: &Row) -> rusqlite::Result<Album> {
    Ok(Album {
        id: r.get(0)?,
        title: r.get(1)?,
        artist_id: r.get(2)?,
        artist: r.get(3)?,
        year: r.get(4)?,
        art_id: r.get(5)?,
        track_count: r.get(6)?,
        duration_ms: r.get(7)?,
        dir: r.get(8)?,
    })
}

const TRACK_SQL: &str = r#"
SELECT t.id, t.title, t.album_id, al.title, t.artist_id, ar.name, aa.name,
       t.disc, t.number, t.duration_ms, t.path, t.codec, t.sample_rate, t.bit_depth,
       t.channels, t.genre, al.year, al.art_id,
       EXISTS (SELECT 1 FROM lyrics l WHERE l.track_id = t.id)
FROM tracks t
JOIN albums al ON al.id = t.album_id
JOIN artists ar ON ar.id = t.artist_id
JOIN artists aa ON aa.id = al.artist_id
"#;

fn track_from_row(r: &Row) -> rusqlite::Result<Track> {
    Ok(Track {
        id: r.get(0)?,
        title: r.get(1)?,
        album_id: r.get(2)?,
        album: r.get(3)?,
        artist_id: r.get(4)?,
        artist: r.get(5)?,
        album_artist: r.get(6)?,
        disc: r.get(7)?,
        number: r.get(8)?,
        duration_ms: r.get(9)?,
        path: r.get(10)?,
        codec: r.get(11)?,
        sample_rate: r.get(12)?,
        bit_depth: r.get(13)?,
        channels: r.get(14)?,
        genre: r.get(15)?,
        year: r.get(16)?,
        art_id: r.get(17)?,
        has_lyrics: r.get::<_, i64>(18)? != 0,
    })
}

/// A pattern for `LIKE` that matches the query anywhere, case-insensitively.
fn like(q: &str) -> String {
    format!("%{}%", q.trim().replace('%', "\\%").replace('_', "\\_"))
}

pub fn stats(conn: &Connection) -> Result<Stats> {
    let one = |sql: &str| -> Result<i64> { Ok(conn.query_row(sql, [], |r| r.get(0))?) };
    Ok(Stats {
        artists: one("SELECT COUNT(*) FROM artists WHERE id IN (SELECT artist_id FROM albums)")?,
        albums: one("SELECT COUNT(*) FROM albums")?,
        tracks: one("SELECT COUNT(*) FROM tracks")?,
        duration_ms: one("SELECT COALESCE(SUM(duration_ms),0) FROM tracks")?,
        bytes: one("SELECT COALESCE(SUM(size),0) FROM tracks")?,
        albums_without_art: one("SELECT COUNT(*) FROM albums WHERE art_id IS NULL")?,
        tracks_without_lyrics: one(
            "SELECT COUNT(*) FROM tracks WHERE id NOT IN (SELECT track_id FROM lyrics)",
        )?,
        last_scan: conn
            .query_row("SELECT value FROM meta WHERE key = 'last_scan'", [], |r| {
                r.get(0)
            })
            .optional()?,
    })
}

pub fn artists(conn: &Connection, q: Option<&str>) -> Result<Vec<Artist>> {
    let sql = r#"
        SELECT ar.id, ar.name,
               (SELECT COUNT(*) FROM albums a WHERE a.artist_id = ar.id),
               (SELECT COUNT(*) FROM tracks t WHERE t.artist_id = ar.id OR t.album_id IN (SELECT id FROM albums a WHERE a.artist_id = ar.id))
        FROM artists ar
        WHERE ar.id IN (SELECT artist_id FROM albums UNION SELECT artist_id FROM tracks)
          AND (?1 IS NULL OR ar.name LIKE ?1 ESCAPE '\')
        ORDER BY ar.sort_name"#;
    let mut st = conn.prepare(sql)?;
    let rows = st.query_map(params![q.map(like)], |r| {
        Ok(Artist {
            id: r.get(0)?,
            name: r.get(1)?,
            album_count: r.get(2)?,
            track_count: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn albums(conn: &Connection, q: Option<&str>, artist_id: Option<i64>) -> Result<Vec<Album>> {
    let sql = format!(
        "{ALBUM_SQL} WHERE (?1 IS NULL OR al.title LIKE ?1 ESCAPE '\\' OR ar.name LIKE ?1 ESCAPE '\\')
           AND (?2 IS NULL OR al.artist_id = ?2)
         ORDER BY ar.sort_name, al.year, al.title"
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map(params![q.map(like), artist_id], album_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn album(conn: &Connection, id: i64) -> Result<Option<Album>> {
    let sql = format!("{ALBUM_SQL} WHERE al.id = ?1");
    Ok(conn.query_row(&sql, [id], album_from_row).optional()?)
}

pub fn tracks(
    conn: &Connection,
    q: Option<&str>,
    album_id: Option<i64>,
    limit: i64,
) -> Result<Vec<Track>> {
    let sql = format!(
        "{TRACK_SQL} WHERE (?1 IS NULL OR t.title LIKE ?1 ESCAPE '\\' OR ar.name LIKE ?1 ESCAPE '\\' OR al.title LIKE ?1 ESCAPE '\\')
           AND (?2 IS NULL OR t.album_id = ?2)
         ORDER BY aa.sort_name, al.year, al.title, t.disc, t.number, t.title
         LIMIT ?3"
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map(params![q.map(like), album_id, limit], track_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn track(conn: &Connection, id: i64) -> Result<Option<Track>> {
    let sql = format!("{TRACK_SQL} WHERE t.id = ?1");
    Ok(conn.query_row(&sql, [id], track_from_row).optional()?)
}

pub fn lyrics(conn: &Connection, track_id: i64) -> Result<Option<Lyrics>> {
    Ok(conn
        .query_row(
            "SELECT track_id, synced, text, source FROM lyrics WHERE track_id = ?1",
            [track_id],
            |r| {
                Ok(Lyrics {
                    track_id: r.get(0)?,
                    synced: r.get::<_, i64>(1)? != 0,
                    text: r.get(2)?,
                    source: r.get(3)?,
                })
            },
        )
        .optional()?)
}

pub fn art(conn: &Connection, id: i64) -> Result<Option<(String, Vec<u8>)>> {
    Ok(conn
        .query_row("SELECT mime, data FROM art WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?)
}

/// A cached small version of an image, if one was made before.
pub fn art_thumb(conn: &Connection, art_id: i64, size: u32) -> Result<Option<Vec<u8>>> {
    Ok(conn
        .query_row(
            "SELECT data FROM art_thumbs WHERE art_id = ?1 AND size = ?2",
            params![art_id, size as i64],
            |r| r.get(0),
        )
        .optional()?)
}

pub fn insert_art_thumb(conn: &Connection, art_id: i64, size: u32, data: &[u8]) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO art_thumbs (art_id, size, data) VALUES (?1, ?2, ?3)",
        params![art_id, size as i64, data],
    )?;
    Ok(())
}

/// Stores an image once and returns its id. The same bytes always give the same id.
pub fn insert_art(conn: &Connection, mime: &str, data: &[u8], source: &str) -> Result<i64> {
    let hash = blake3::hash(data).to_hex().to_string();
    conn.execute(
        "INSERT OR IGNORE INTO art (hash, mime, data, source) VALUES (?1, ?2, ?3, ?4)",
        params![hash, mime, data, source],
    )?;
    Ok(conn.query_row("SELECT id FROM art WHERE hash = ?1", [hash], |r| r.get(0))?)
}

pub fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    iso_from_secs(secs)
}

/// Simple UTC formatting without a date crate.
fn iso_from_secs(secs: u64) -> String {
    let days = secs / 86400;
    let (h, m, s) = ((secs % 86400) / 3600, (secs % 3600) / 60, secs % 60);
    let (y, mo, d) = civil_from_days(days as i64);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

// Howard Hinnant's algorithm: days since 1970-01-01 to a civil date.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---- playlists ---------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub track_count: i64,
    pub duration_ms: i64,
    pub created_at: String,
}

const PLAYLIST_SQL: &str = r#"
SELECT p.id, p.name,
       (SELECT COUNT(*) FROM playlist_tracks pt WHERE pt.playlist_id = p.id),
       (SELECT COALESCE(SUM(t.duration_ms), 0) FROM playlist_tracks pt JOIN tracks t ON t.id = pt.track_id WHERE pt.playlist_id = p.id),
       p.created_at
FROM playlists p
"#;

fn playlist_from_row(r: &Row) -> rusqlite::Result<Playlist> {
    Ok(Playlist {
        id: r.get(0)?,
        name: r.get(1)?,
        track_count: r.get(2)?,
        duration_ms: r.get(3)?,
        created_at: r.get(4)?,
    })
}

pub fn playlists(conn: &Connection) -> Result<Vec<Playlist>> {
    let sql = format!("{PLAYLIST_SQL} ORDER BY p.name COLLATE NOCASE");
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map([], playlist_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn playlist(conn: &Connection, id: i64) -> Result<Option<Playlist>> {
    let sql = format!("{PLAYLIST_SQL} WHERE p.id = ?1");
    Ok(conn.query_row(&sql, [id], playlist_from_row).optional()?)
}

/// A playlist by its exact name, case-insensitively.
pub fn playlist_by_name(conn: &Connection, name: &str) -> Result<Option<Playlist>> {
    let sql = format!("{PLAYLIST_SQL} WHERE p.name = ?1 COLLATE NOCASE");
    Ok(conn
        .query_row(&sql, [name.trim()], playlist_from_row)
        .optional()?)
}

pub fn create_playlist(conn: &Connection, name: &str) -> Result<Playlist> {
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("a playlist needs a name");
    }
    if playlist_by_name(conn, name)?.is_some() {
        anyhow::bail!("there is already a playlist called {name}");
    }
    conn.execute("INSERT INTO playlists (name) VALUES (?1)", [name])?;
    let id = conn.last_insert_rowid();
    Ok(playlist(conn, id)?.expect("just inserted"))
}

pub fn rename_playlist(conn: &Connection, id: i64, name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("a playlist needs a name");
    }
    if let Some(other) = playlist_by_name(conn, name)? {
        if other.id != id {
            anyhow::bail!("there is already a playlist called {name}");
        }
    }
    conn.execute(
        "UPDATE playlists SET name = ?1 WHERE id = ?2",
        params![name, id],
    )?;
    Ok(())
}

pub fn delete_playlist(conn: &Connection, id: i64) -> Result<bool> {
    conn.execute("DELETE FROM playlist_tracks WHERE playlist_id = ?1", [id])?;
    Ok(conn.execute("DELETE FROM playlists WHERE id = ?1", [id])? > 0)
}

/// The tracks of a playlist in order.
pub fn playlist_tracks(conn: &Connection, id: i64) -> Result<Vec<Track>> {
    let sql = format!(
        "{TRACK_SQL} JOIN playlist_tracks pt ON pt.track_id = t.id
         WHERE pt.playlist_id = ?1 ORDER BY pt.position"
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map([id], track_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Appends tracks that are not in the list yet. Returns how many went in.
pub fn add_to_playlist(conn: &Connection, id: i64, track_ids: &[i64]) -> Result<usize> {
    let mut next: i64 = conn.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM playlist_tracks WHERE playlist_id = ?1",
        [id],
        |r| r.get(0),
    )?;
    let mut added = 0;
    for t in track_ids {
        let present: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM playlist_tracks WHERE playlist_id = ?1 AND track_id = ?2)",
            params![id, t],
            |r| r.get::<_, i64>(0).map(|x| x != 0),
        )?;
        if present {
            continue;
        }
        let n = conn.execute(
            "INSERT INTO playlist_tracks (playlist_id, position, track_id)
             SELECT ?1, ?2, id FROM tracks WHERE id = ?3",
            params![id, next, t],
        )?;
        if n > 0 {
            next += 1;
            added += 1;
        }
    }
    Ok(added)
}

/// Removes the entry at `position` (0-based, as `playlist_tracks` lists them)
/// and closes the gap.
pub fn remove_from_playlist(conn: &Connection, id: i64, position: usize) -> Result<bool> {
    let ids: Vec<i64> = {
        let mut st = conn.prepare(
            "SELECT track_id FROM playlist_tracks WHERE playlist_id = ?1 ORDER BY position",
        )?;
        let rows = st.query_map([id], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    if position >= ids.len() {
        return Ok(false);
    }
    let mut ids = ids;
    ids.remove(position);
    set_playlist_tracks(conn, id, &ids)?;
    Ok(true)
}

/// Replaces the whole list, for reordering.
pub fn set_playlist_tracks(conn: &Connection, id: i64, track_ids: &[i64]) -> Result<()> {
    conn.execute("DELETE FROM playlist_tracks WHERE playlist_id = ?1", [id])?;
    for (i, t) in track_ids.iter().enumerate() {
        conn.execute(
            "INSERT INTO playlist_tracks (playlist_id, position, track_id) VALUES (?1, ?2, ?3)",
            params![id, i as i64, t],
        )?;
    }
    Ok(())
}

/// Ids of the playlists a track is in.
pub fn playlists_of_track(conn: &Connection, track_id: i64) -> Result<Vec<i64>> {
    let mut st = conn.prepare(
        "SELECT DISTINCT playlist_id FROM playlist_tracks WHERE track_id = ?1 ORDER BY playlist_id",
    )?;
    let rows = st.query_map([track_id], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

// ---- radio -------------------------------------------------------------

/// Picks one track for the radio: at random from `playlist` (or the whole
/// library), not played in the last `rest_days`, not in `avoid`, and by a
/// different artist than `last_artist` when there is a choice. When the rest
/// rule leaves nothing, it is relaxed, then the artist rule, so the radio
/// never falls silent while there is music.
pub fn radio_pick(
    conn: &Connection,
    playlist: Option<i64>,
    rest_days: i64,
    avoid: &[i64],
    last_artist: Option<i64>,
) -> Result<Option<i64>> {
    let avoid_list = avoid
        .iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let avoid_sql = if avoid_list.is_empty() {
        String::new()
    } else {
        format!(" AND t.id NOT IN ({avoid_list})")
    };
    let pool_sql = match playlist {
        Some(_) => " AND t.id IN (SELECT track_id FROM playlist_tracks WHERE playlist_id = ?1)",
        None => " AND ?1 IS NULL",
    };
    let sql = format!(
        "SELECT t.id FROM tracks t WHERE t.duration_ms > 0{pool_sql}{avoid_sql}
           AND t.id NOT IN (SELECT track_id FROM plays WHERE started_at > ?2)
           AND (?3 IS NULL OR t.artist_id != ?3)
         ORDER BY RANDOM() LIMIT 1"
    );
    let since = days_ago_iso(rest_days);
    // Nothing was started after the year 9999: the rest rule switched off.
    let never = "9999".to_string();
    for (rest, artist) in [(true, true), (true, false), (false, true), (false, false)] {
        let since = if rest { &since } else { &never };
        let artist = if artist { last_artist } else { None };
        let pick: Option<i64> = conn
            .query_row(&sql, params![playlist, since, artist], |r| r.get(0))
            .optional()?;
        if pick.is_some() {
            return Ok(pick);
        }
    }
    Ok(None)
}

/// `now_iso` moved back by `days`.
fn days_ago_iso(days: i64) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
        - days.max(0) * 86_400;
    iso_from_secs(secs.max(0) as u64)
}

// ---- picks -------------------------------------------------------------

/// One album on the Home shelf, and why it is there.
#[derive(Debug, Clone, Serialize)]
pub struct Pick {
    #[serde(flatten)]
    pub album: Album,
    /// "same genre", "same artist", "wildcard" or "same century".
    pub why: String,
}

/// The genre most of an album's tracks carry.
pub fn album_genre(conn: &Connection, album_id: i64) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT genre FROM tracks WHERE album_id = ?1 AND genre IS NOT NULL AND genre <> ''
             GROUP BY genre ORDER BY COUNT(*) DESC, genre LIMIT 1",
            [album_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// Up to four albums to go with `album_id`, each different and none of them
/// that album: one of the same genre by someone else, another by the same
/// artist (or a second of the genre when there is none), one at random, and
/// one from the same century. A rule that finds nothing is left out.
pub fn picks(conn: &Connection, album_id: i64) -> Result<Vec<Pick>> {
    let Some(base) = album(conn, album_id)? else {
        return Ok(Vec::new());
    };
    let genre = album_genre(conn, album_id)?;
    let mut taken = vec![album_id];
    let mut out = Vec::new();

    fn pick(
        conn: &Connection,
        cond: &str,
        args: &[&dyn rusqlite::ToSql],
        why: &str,
        taken: &mut Vec<i64>,
        out: &mut Vec<Pick>,
    ) -> Result<bool> {
        let not_in = taken
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "{ALBUM_SQL} WHERE al.id NOT IN ({not_in}) AND ({cond}) ORDER BY random() LIMIT 1"
        );
        let found = conn.query_row(&sql, args, album_from_row).optional()?;
        Ok(match found {
            Some(a) => {
                taken.push(a.id);
                out.push(Pick {
                    album: a,
                    why: why.to_string(),
                });
                true
            }
            None => false,
        })
    }
    const GENRE: &str = "al.id IN (SELECT album_id FROM tracks WHERE genre = ?1)";

    if let Some(g) = &genre {
        let other = format!("{GENRE} AND al.artist_id <> ?2");
        if !pick(
            conn,
            &other,
            &[g, &base.artist_id],
            "same genre",
            &mut taken,
            &mut out,
        )? {
            pick(conn, GENRE, &[g], "same genre", &mut taken, &mut out)?;
        }
    }
    if !pick(
        conn,
        "al.artist_id = ?1",
        &[&base.artist_id],
        "same artist",
        &mut taken,
        &mut out,
    )? {
        if let Some(g) = &genre {
            pick(conn, GENRE, &[g], "same genre", &mut taken, &mut out)?;
        }
    }
    // The century goes before the wildcard so the wildcard cannot take the
    // only album that fits it; the shelf still shows the wildcard third.
    if let Some(y) = base.year {
        pick(
            conn,
            "al.year IS NOT NULL AND al.year / 100 = ?1 / 100",
            &[&y],
            "same century",
            &mut taken,
            &mut out,
        )?;
    }
    let century = out.iter().position(|p| p.why == "same century");
    pick(conn, "1", &[], "wildcard", &mut taken, &mut out)?;
    if let (Some(c), Some(w)) = (century, out.iter().position(|p| p.why == "wildcard")) {
        out.swap(c, w);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(tag: &str) -> (Connection, std::path::PathBuf) {
        let path =
            std::env::temp_dir().join(format!("katalog-picks-{tag}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let conn = open(&path).unwrap();
        conn.execute_batch(
            "INSERT INTO artists (id, name, sort_name) VALUES (1, 'Azra', 'Azra'), (2, 'Joy Division', 'Joy Division'), (3, 'Radiohead', 'Radiohead');
             INSERT INTO albums (id, artist_id, title, year, dir) VALUES
               (1, 1, 'Sunčana strana ulice', 1981, 'a'), (2, 1, 'Filigranski pločnici', 1982, 'b'),
               (3, 2, 'Closer', 1980, 'c'), (4, 3, 'Kid A', 2000, 'd'), (5, 3, 'OK Computer', 1997, 'e');",
        )
        .unwrap();
        for (id, album, artist, genre) in [
            (1, 1, 1, "New Wave"),
            (2, 2, 1, "New Wave"),
            (3, 3, 2, "New Wave"),
            (4, 4, 3, "Electronic"),
            (5, 5, 3, "Rock"),
        ] {
            conn.execute(
                "INSERT INTO tracks (id, album_id, artist_id, title, path, size, mtime, codec, genre) VALUES (?1, ?2, ?3, 't', ?4, 0, 0, 'flac', ?5)",
                params![id, album, artist, format!("/{id}.flac"), genre],
            )
            .unwrap();
        }
        (conn, path)
    }

    #[test]
    fn picks_follow_the_rules() {
        let (conn, path) = library("rules");
        let p = picks(&conn, 1).unwrap();
        let why: Vec<&str> = p.iter().map(|p| p.why.as_str()).collect();
        assert_eq!(
            why,
            ["same genre", "same artist", "wildcard", "same century"]
        );
        assert_eq!(p[0].album.id, 3, "same genre by someone else");
        assert_eq!(p[1].album.id, 2, "the other Azra album");
        let ids: std::collections::HashSet<i64> = p.iter().map(|p| p.album.id).collect();
        assert_eq!(ids.len(), 4);
        assert!(!ids.contains(&1));
        assert_eq!(p[3].album.year.unwrap() / 100, 19);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn no_other_album_by_the_artist_falls_back_to_the_genre() {
        let (conn, path) = library("fallback");
        conn.execute("UPDATE tracks SET genre = 'Rock' WHERE id = 4", [])
            .unwrap();
        let p = picks(&conn, 3).unwrap();
        assert_eq!(p[0].why, "same genre");
        assert_eq!(p[1].why, "same genre");
        let _ = std::fs::remove_file(path);
    }
}
