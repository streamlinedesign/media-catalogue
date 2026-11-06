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

pub const USER_AGENT: &str = concat!(
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
    /// Films and shows TMDB knew, and those it did not.
    pub video_found: usize,
    pub video_missing: usize,
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
    save_lyrics(conn, track.id, &track.path, text, "lrclib", write_lrc)
}

/// Saves lyrics for a track, replacing what was there (a hand-picked search
/// hit wins over an earlier automatic one), and writes the `.lrc` when asked.
pub fn save_lyrics(
    conn: &Connection,
    track_id: i64,
    path: &str,
    text: &str,
    source: &str,
    write_lrc: bool,
) -> Result<()> {
    let synced = lyrics::is_synced(text);
    conn.execute(
        "INSERT INTO lyrics (track_id, synced, text, source) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(track_id) DO UPDATE SET synced = excluded.synced, text = excluded.text, source = excluded.source",
        params![track_id, synced as i64, text, source],
    )?;
    conn.execute(
        "UPDATE tracks SET lyrics_tried = 0 WHERE id = ?1",
        [track_id],
    )?;
    if write_lrc {
        if let Err(err) = lyrics::write_sidecar(Path::new(path), text) {
            warn!("could not write .lrc beside {path}: {err}");
        }
    }
    Ok(())
}

/// Takes a track's lyrics away (wrong words, bad timing) and keeps them
/// away: neither the online lookup nor a rescan of an embedded tag or a
/// .lrc brings them back. Picking lyrics by hand again lifts that. Files
/// are left as they are.
pub fn remove_lyrics(conn: &Connection, track_id: i64) -> Result<()> {
    conn.execute("DELETE FROM lyrics WHERE track_id = ?1", [track_id])?;
    conn.execute(
        "UPDATE tracks SET lyrics_tried = ?2 WHERE id = ?1",
        params![track_id, LYRICS_REMOVED],
    )?;
    Ok(())
}

/// `tracks.lyrics_tried` for lyrics taken away by hand (1 is "nothing found").
pub const LYRICS_REMOVED: i64 = 2;

/// Forget earlier "nothing found" results so every album is looked up again.
pub fn reset_art_tried(conn: &Connection) -> Result<usize> {
    Ok(conn.execute("UPDATE albums SET art_tried = 0 WHERE art_id IS NULL", [])?)
}

/// Forget earlier "nothing found" results so every track is looked up again.
pub fn reset_lyrics_tried(conn: &Connection) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE tracks SET lyrics_tried = 0
         WHERE lyrics_tried = 1 AND id NOT IN (SELECT track_id FROM lyrics)",
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
    /// Lyrics from LRCLIB. Tries the exact match first, then the same with the
    /// title and album stripped of "(Remastered 2009)", "[Live]", "feat." and the
    /// like, then a free search where the best hit by duration and title wins.
    /// Returns None only when none of that finds anything.
    pub async fn lrclib(
        &self,
        artist: &str,
        title: &str,
        album: &str,
        duration_ms: i64,
    ) -> Result<Option<String>> {
        let secs = (duration_ms as f64 / 1000.0).round() as i64;
        let c_artist = clean_artist(artist);
        let c_title = clean_title(title);
        let c_album = clean_title(album);

        // 1. Exact, then cleaned, then without the album (albums differ most between tags and LRCLIB).
        let mut attempts: Vec<(String, String, Option<String>)> = vec![(
            artist.to_string(),
            title.to_string(),
            Some(album.to_string()),
        )];
        for cand in [
            (c_artist.clone(), c_title.clone(), Some(c_album.clone())),
            (artist.to_string(), title.to_string(), None),
            (c_artist.clone(), c_title.clone(), None),
        ] {
            if !attempts.contains(&cand) {
                attempts.push(cand);
            }
        }
        for (a, t, al) in &attempts {
            if let Some(hit) = self.lrclib_get(a, t, al.as_deref(), secs).await? {
                return Ok(Some(hit));
            }
        }

        // 2. Search and pick the closest.
        for (a, t) in [(artist, title), (c_artist.as_str(), c_title.as_str())] {
            let hits = self.lrclib_search(a, t).await?;
            if let Some(text) = pick_hit(hits, &c_title, duration_ms) {
                return Ok(Some(text));
            }
        }
        Ok(None)
    }

    async fn lrclib_get(
        &self,
        artist: &str,
        title: &str,
        album: Option<&str>,
        secs: i64,
    ) -> Result<Option<String>> {
        let mut url = format!(
            "https://lrclib.net/api/get?artist_name={}&track_name={}&duration={secs}",
            urlencoding::encode(artist),
            urlencoding::encode(title),
        );
        if let Some(al) = album {
            url.push_str(&format!("&album_name={}", urlencoding::encode(al)));
        }
        let res = self.http.get(&url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let hit: LrcHit = res
            .error_for_status()?
            .json()
            .await
            .context("lrclib json")?;
        Ok(hit.text())
    }

    /// A free search on LRCLIB, for picking lyrics by hand. `query` is whatever
    /// the user typed; `artist` narrows it when given.
    pub async fn search_lyrics(&self, query: &str, artist: Option<&str>) -> Result<Vec<LyricsHit>> {
        let mut url = format!(
            "https://lrclib.net/api/search?q={}",
            urlencoding::encode(query.trim())
        );
        if let Some(a) = artist.map(str::trim).filter(|a| !a.is_empty()) {
            url.push_str(&format!("&artist_name={}", urlencoding::encode(a)));
        }
        let res = self.http.get(&url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        let hits: Vec<LrcHit> = res
            .error_for_status()?
            .json()
            .await
            .context("lrclib search json")?;
        Ok(hits
            .into_iter()
            .filter_map(|h| {
                let synced = h
                    .synced
                    .as_deref()
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false);
                let meta = (
                    h.track_name.clone(),
                    h.artist_name.clone(),
                    h.album_name.clone(),
                    h.duration,
                );
                h.text().map(|text| LyricsHit {
                    source: "lrclib",
                    title: meta.0,
                    artist: meta.1,
                    album: meta.2,
                    duration_ms: meta.3.map(|d| (d * 1000.0) as i64),
                    synced,
                    text,
                })
            })
            .collect())
    }

    async fn lrclib_search(&self, artist: &str, title: &str) -> Result<Vec<LrcHit>> {
        let url = format!(
            "https://lrclib.net/api/search?artist_name={}&track_name={}",
            urlencoding::encode(artist),
            urlencoding::encode(title),
        );
        let res = self.http.get(&url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        res.error_for_status()?
            .json()
            .await
            .context("lrclib search json")
    }
}

/// One search result, with its lyrics, ready to be shown and picked.
#[derive(Debug, Clone, Serialize)]
pub struct LyricsHit {
    pub source: &'static str,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: Option<i64>,
    pub synced: bool,
    pub text: String,
}

#[derive(Deserialize)]
struct LrcHit {
    #[serde(rename = "trackName", default)]
    track_name: String,
    #[serde(rename = "artistName", default)]
    artist_name: String,
    #[serde(rename = "albumName", default)]
    album_name: String,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(rename = "syncedLyrics")]
    synced: Option<String>,
    #[serde(rename = "plainLyrics")]
    plain: Option<String>,
    instrumental: Option<bool>,
}

impl LrcHit {
    fn text(self) -> Option<String> {
        if self.instrumental == Some(true) {
            return Some("[instrumental]".into());
        }
        self.synced
            .filter(|s| !s.trim().is_empty())
            .or(self.plain.filter(|s| !s.trim().is_empty()))
    }
}

/// The best search hit: same (cleaned) title, duration within five seconds,
/// synced before plain. Nothing that does not match the title.
fn pick_hit(hits: Vec<LrcHit>, want_title: &str, duration_ms: i64) -> Option<String> {
    let want = normalize(want_title);
    let mut best: Option<(i64, LrcHit)> = None;
    for h in hits {
        if normalize(&clean_title(&h.track_name)) != want {
            continue;
        }
        let diff = h
            .duration
            .map(|d| ((d * 1000.0) as i64 - duration_ms).abs())
            .unwrap_or(i64::MAX);
        if duration_ms > 0 && diff > 5000 {
            continue;
        }
        // Lower score wins: duration difference, plus a penalty for plain-only.
        let score = diff
            + if h
                .synced
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
            {
                0
            } else {
                2000
            };
        if best.as_ref().map(|(s, _)| score < *s).unwrap_or(true) {
            best = Some((score, h));
        }
    }
    best.and_then(|(_, h)| h.text())
}

/// Drops the bits tags carry and lyric databases do not: "(Remastered 2009)",
/// "[Live at ...]", " - 2011 Remaster", "(feat. X)", "(Deluxe Edition)".
pub fn clean_title(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for ch in s.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    // " - 2009 Remaster", " - Live", " - Radio Edit"
    let lowered = out.to_ascii_lowercase();
    for marker in [
        " - remaster",
        " - live",
        " - radio edit",
        " - single version",
        " - mono",
        " - stereo",
        " - demo",
        " - bonus track",
        " - acoustic",
        " - edit",
        " - version",
    ] {
        if let Some(i) = lowered.find(marker) {
            out.truncate(i);
            break;
        }
    }
    // " - 2009 Remaster" style with a year in front
    if let Some(i) = out.rfind(" - ") {
        let tail = out[i + 3..].to_ascii_lowercase();
        if tail.contains("remaster") || tail.contains("mix") || tail.contains("edition") {
            out.truncate(i);
        }
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.is_empty() {
        s.trim().to_string()
    } else {
        out
    }
}

/// "Artist feat. Someone" → "Artist"; "A & B" and "A, B" are kept.
pub fn clean_artist(s: &str) -> String {
    let lowered = s.to_ascii_lowercase();
    let cut = [
        " feat. ",
        " feat ",
        " ft. ",
        " ft ",
        " featuring ",
        " with ",
    ]
    .iter()
    .filter_map(|m| lowered.find(m))
    .min();
    let base = match cut {
        Some(i) if i > 0 => &s[..i],
        _ => s,
    };
    let base = clean_title(base);
    if base.is_empty() {
        s.trim().to_string()
    } else {
        base
    }
}

/// Lowercase letters and digits only, so "Don't Stop" and "Dont stop" compare equal.
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_lose_their_brackets_and_remaster_tails() {
        assert_eq!(
            clean_title("Paranoid Android (Remastered 2009)"),
            "Paranoid Android"
        );
        assert_eq!(clean_title("Starless - 2011 Remaster"), "Starless");
        assert_eq!(
            clean_title("Disorder [Live at Les Bains Douches]"),
            "Disorder"
        );
        assert_eq!(clean_title("OK Computer (Deluxe Edition)"), "OK Computer");
        assert_eq!(clean_title("(Untitled)"), "(Untitled)");
        assert_eq!(
            clean_title("21st Century Schizoid Man"),
            "21st Century Schizoid Man"
        );
    }

    #[test]
    fn artists_lose_features() {
        assert_eq!(
            clean_artist("Massive Attack feat. Elizabeth Fraser"),
            "Massive Attack"
        );
        assert_eq!(clean_artist("Simon & Garfunkel"), "Simon & Garfunkel");
        assert_eq!(clean_artist("Azra"), "Azra");
    }

    #[test]
    fn search_hits_are_picked_by_title_and_duration() {
        let hit = |name: &str, dur: f64, synced: bool| LrcHit {
            track_name: name.into(),
            artist_name: String::new(),
            album_name: String::new(),
            duration: Some(dur),
            synced: synced.then(|| "[00:01.00]x".to_string()),
            plain: Some("x".into()),
            instrumental: None,
        };
        let hits = vec![
            hit("Airbag (Live)", 290.0, true),
            hit("Airbag", 284.0, false),
            hit("Airbag", 285.0, true),
            hit("Something Else", 285.0, true),
        ];
        assert_eq!(
            pick_hit(hits, "Airbag", 285_000).as_deref(),
            Some("[00:01.00]x")
        );
        let far = vec![hit("Airbag", 200.0, true)];
        assert_eq!(pick_hit(far, "Airbag", 285_000), None);
    }
}
