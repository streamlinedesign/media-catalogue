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
    // Simple UTC formatting without a date crate.
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
