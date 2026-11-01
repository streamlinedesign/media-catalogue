//! Online lookups for what the files don't carry: album art from the Cover Art
//! Archive (via MusicBrainz) and lyrics from LRCLIB. Both are free and need no key.
//!
//! Results go into the database and, for lyrics, into a `.lrc` next to the file.
//! Every album and track is tried once; a miss is remembered so it is not retried
//! on every start.

use crate::{db, lyrics};
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;
use tracing::warn;

const USER_AGENT: &str = concat!(
    "Katalog/",
    env!("CARGO_PKG_VERSION"),
    " (personal music library)"
);

#[derive(Debug, Default, Clone, Serialize)]
pub struct FetchReport {
    pub art_found: usize,
    pub art_missing: usize,
    pub lyrics_found: usize,
    pub lyrics_missing: usize,
    /// Lookups that errored (network, bad answer). These are retried next time.
    pub failed: usize,
}

pub struct Fetcher {
    http: reqwest::Client,
}

impl Fetcher {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(20))
            .build()?;
        Ok(Self { http })
    }
}

/// An album still without art, as the daemon needs it for a lookup.
#[derive(Debug, Clone)]
pub struct PendingAlbum {
    pub id: i64,
    pub artist: String,
    pub title: String,
}

/// A track still without lyrics.
#[derive(Debug, Clone)]
pub struct PendingTrack {
    pub id: i64,
    pub artist: String,
    pub title: String,
    pub album: String,
    pub duration_ms: i64,
    pub path: String,
}

/// Albums with no art that have not been looked up yet.
pub fn pending_albums(conn: &Connection, limit: usize) -> Result<Vec<PendingAlbum>> {
    let mut st = conn.prepare(
        "SELECT al.id, ar.name, al.title FROM albums al JOIN artists ar ON ar.id = al.artist_id
         WHERE al.art_id IS NULL AND al.art_tried = 0 ORDER BY al.id LIMIT ?1",
    )?;
    let rows = st.query_map([limit as i64], |r| {
        Ok(PendingAlbum {
            id: r.get(0)?,
            artist: r.get(1)?,
            title: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Tracks with no lyrics that have not been looked up yet.
pub fn pending_tracks(conn: &Connection, limit: usize) -> Result<Vec<PendingTrack>> {
    let mut st = conn.prepare(
        "SELECT t.id, ar.name, t.title, al.title, t.duration_ms, t.path
         FROM tracks t JOIN artists ar ON ar.id = t.artist_id JOIN albums al ON al.id = t.album_id
         WHERE t.lyrics_tried = 0 AND t.id NOT IN (SELECT track_id FROM lyrics) ORDER BY t.id LIMIT ?1",
    )?;
    let rows = st.query_map([limit as i64], |r| {
        Ok(PendingTrack {
            id: r.get(0)?,
            artist: r.get(1)?,
            title: r.get(2)?,
            album: r.get(3)?,
            duration_ms: r.get(4)?,
            path: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn store_art(conn: &Connection, album_id: i64, mime: &str, data: &[u8]) -> Result<()> {
    let art_id = db::insert_art(conn, mime, data, "coverartarchive")?;
    conn.execute(
        "UPDATE albums SET art_id = ?1 WHERE id = ?2",
        params![art_id, album_id],
    )?;
    Ok(())
}

pub fn mark_art_missing(conn: &Connection, album_id: i64) -> Result<()> {
    conn.execute("UPDATE albums SET art_tried = 1 WHERE id = ?1", [album_id])?;
    Ok(())
}

/// Stores fetched lyrics and, when asked, writes them as a `.lrc` beside the file.
pub fn store_lyrics(
    conn: &Connection,
    track: &PendingTrack,
    text: &str,
    write_lrc: bool,
) -> Result<()> {
    let synced = lyrics::is_synced(text);
    conn.execute(
        "INSERT INTO lyrics (track_id, synced, text, source) VALUES (?1, ?2, ?3, 'lrclib')
         ON CONFLICT(track_id) DO NOTHING",
        params![track.id, synced as i64, text],
    )?;
    if write_lrc {
        if let Err(err) = lyrics::write_sidecar(Path::new(&track.path), text) {
            warn!("could not write .lrc beside {}: {err}", track.path);
        }
    }
    Ok(())
}

/// Forget earlier "nothing found" results so every album is looked up again.
pub fn reset_art_tried(conn: &Connection) -> Result<usize> {
    Ok(conn.execute("UPDATE albums SET art_tried = 0 WHERE art_id IS NULL", [])?)
}

/// Forget earlier "nothing found" results so every track is looked up again.
pub fn reset_lyrics_tried(conn: &Connection) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE tracks SET lyrics_tried = 0 WHERE id NOT IN (SELECT track_id FROM lyrics)",
        [],
    )?)
}

pub fn mark_lyrics_missing(conn: &Connection, track_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE tracks SET lyrics_tried = 1 WHERE id = ?1",
        [track_id],
    )?;
    Ok(())
}

impl Fetcher {
    /// Front cover for an album via MusicBrainz release-group search and the Cover Art Archive.
    pub async fn cover_art(&self, artist: &str, album: &str) -> Result<Option<(String, Vec<u8>)>> {
        #[derive(Deserialize)]
        struct Search {
            #[serde(rename = "release-groups", default)]
            groups: Vec<Group>,
        }
        #[derive(Deserialize)]
        struct Group {
            id: String,
        }
        let q = format!("releasegroup:{} AND artist:{}", quote(album), quote(artist));
        let url = format!(
            "https://musicbrainz.org/ws/2/release-group/?fmt=json&limit=3&query={}",
            urlencoding::encode(&q)
        );
        let search: Search = self
            .http
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("musicbrainz json")?;
        for g in search.groups {
            let url = format!(
                "https://coverartarchive.org/release-group/{}/front-500",
                g.id
            );
            let res = self.http.get(&url).send().await?;
            if res.status().is_success() {
                let mime = res
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("image/jpeg")
                    .to_string();
                let data = res.bytes().await?.to_vec();
                if !data.is_empty() {
                    return Ok(Some((mime, data)));
                }
            }
        }
        Ok(None)
    }

    /// Lyrics from LRCLIB, synced when they have them, plain otherwise.
    pub async fn lrclib(
        &self,
        artist: &str,
        title: &str,
        album: &str,
        duration_ms: i64,
    ) -> Result<Option<String>> {
        #[derive(Deserialize)]
        struct Hit {
            #[serde(rename = "syncedLyrics")]
            synced: Option<String>,
            #[serde(rename = "plainLyrics")]
            plain: Option<String>,
            instrumental: Option<bool>,
        }
        let url = format!(
            "https://lrclib.net/api/get?artist_name={}&track_name={}&album_name={}&duration={}",
            urlencoding::encode(artist),
            urlencoding::encode(title),
            urlencoding::encode(album),
            (duration_ms as f64 / 1000.0).round() as i64
        );
        let res = self.http.get(&url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let hit: Hit = res
            .error_for_status()?
            .json()
            .await
            .context("lrclib json")?;
        if hit.instrumental == Some(true) {
            return Ok(Some("[instrumental]".into()));
        }
        Ok(hit
            .synced
            .filter(|s| !s.trim().is_empty())
            .or(hit.plain.filter(|s| !s.trim().is_empty())))
    }
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}
