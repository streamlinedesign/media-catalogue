//! Walks the music folder and brings the database in line with it.
//!
//! Unchanged files (same size and mtime) are skipped, so a rescan of 25 GB
//! is a directory walk, not a re-read. Removed files are removed from the
//! database too, and albums left without tracks disappear with them.

use crate::db;
use crate::lyrics;
use anyhow::{Context, Result};
use lofty::picture::PictureType;
use lofty::prelude::*;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};
use tracing::{debug, info, warn};
use walkdir::WalkDir;

#[derive(Debug, Default, Clone, Serialize)]
pub struct ScanReport {
    pub seen: usize,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub unreadable: usize,
    pub seconds: f64,
    /// Films and TV, when they were scanned too.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video: Option<crate::video::VideoReport>,
    /// Books and comics, when they were scanned too.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub books: Option<crate::books::BooksReport>,
}

pub struct Scanner<'a> {
    pub conn: &'a mut Connection,
    pub root: PathBuf,
}

struct FileTags {
    title: String,
    artist: String,
    album_artist: String,
    album: String,
    year: Option<i64>,
    disc: Option<i64>,
    number: Option<i64>,
    genre: Option<String>,
    duration_ms: i64,
    codec: String,
    sample_rate: Option<i64>,
    bit_depth: Option<i64>,
    channels: Option<i64>,
    picture: Option<(String, Vec<u8>)>,
    lyrics: Option<String>,
}

impl<'a> Scanner<'a> {
    pub fn new(conn: &'a mut Connection, root: impl Into<PathBuf>) -> Self {
        Self {
            conn,
            root: root.into(),
        }
    }

    /// Scan the whole root.
    pub fn scan_all(&mut self) -> Result<ScanReport> {
        let root = self.root.clone();
        self.scan_dirs(&[root], true)
    }

    /// Scan only these folders (used by the watcher). With `prune`, tracks under
    /// those folders whose files are gone are removed.
    pub fn scan_dirs(&mut self, dirs: &[PathBuf], prune: bool) -> Result<ScanReport> {
        let started = Instant::now();
        let mut report = ScanReport::default();
        let mut seen_paths: HashSet<String> = HashSet::new();

        for dir in dirs {
            if !dir.exists() {
                continue;
            }
            for entry in WalkDir::new(dir)
                .follow_links(true)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if !entry.file_type().is_file() || !crate::is_audio_path(path) {
                    continue;
                }
                report.seen += 1;
                let key = path.to_string_lossy().into_owned();
                seen_paths.insert(key.clone());
                match self.scan_file(path) {
                    Ok(Outcome::Unchanged) => {}
                    Ok(Outcome::Added) => report.added += 1,
                    Ok(Outcome::Updated) => report.updated += 1,
                    Err(err) => {
                        report.unreadable += 1;
                        warn!("{}: {err:#}", path.display());
                    }
                }
            }
        }

        if prune {
            report.removed = self.prune(dirs, &seen_paths)?;
        }
        self.remove_empty_albums()?;
        self.attach_folder_art(dirs)?;
        if report.added + report.updated + report.removed > 0
            || db::get_meta(self.conn, "genre_playlists")?.is_none()
        {
            let n = db::sync_genre_playlists(self.conn)?;
            db::set_meta(self.conn, "genre_playlists", &n.to_string())?;
        }
        db::set_meta(self.conn, "last_scan", &db::now_iso())?;
        report.seconds = started.elapsed().as_secs_f64();
        info!(
            "scan done: {} files, {} added, {} updated, {} removed, {} unreadable, {:.1}s",
            report.seen,
            report.added,
            report.updated,
            report.removed,
            report.unreadable,
            report.seconds
        );
        Ok(report)
    }

    /// A cover.jpg dropped into a folder after its tracks were scanned: the
    /// tracks are unchanged and skipped, so look for folder art separately for
    /// every album under `dirs` that still has none.
    fn attach_folder_art(&mut self, dirs: &[PathBuf]) -> Result<()> {
        let candidates: Vec<(i64, String)> = {
            let mut st = self.conn.prepare(
                "SELECT al.id, MIN(t.path) FROM albums al JOIN tracks t ON t.album_id = al.id
                 WHERE al.art_id IS NULL GROUP BY al.id",
            )?;
            let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for (album_id, path) in candidates {
            let path = Path::new(&path);
            if !dirs.iter().any(|d| path.starts_with(d)) {
                continue;
            }
            let Some(dir) = path.parent() else { continue };
            if let Some((mime, data)) = folder_art(dir) {
                let art_id = db::insert_art(self.conn, &mime, &data, "folder")?;
                self.conn.execute(
                    "UPDATE albums SET art_id = ?1, art_tried = 0 WHERE id = ?2",
                    params![art_id, album_id],
                )?;
                info!("folder art for album {album_id}: {}", dir.display());
            }
        }
        Ok(())
    }

    fn prune(&mut self, dirs: &[PathBuf], seen: &HashSet<String>) -> Result<usize> {
        let mut stale = Vec::new();
        {
            let mut st = self.conn.prepare("SELECT id, path FROM tracks")?;
            for row in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
                let (id, path) = row?;
                let under = dirs.iter().any(|d| Path::new(&path).starts_with(d));
                if under && !seen.contains(&path) {
                    stale.push(id);
                }
            }
        }
        for id in &stale {
            self.conn
                .execute("DELETE FROM tracks WHERE id = ?1", [id])?;
        }
        Ok(stale.len())
    }

    fn remove_empty_albums(&mut self) -> Result<()> {
        self.conn.execute(
            "DELETE FROM albums WHERE id NOT IN (SELECT DISTINCT album_id FROM tracks)",
            [],
        )?;
        Ok(())
    }

    fn scan_file(&mut self, path: &Path) -> Result<Outcome> {
        let meta = std::fs::metadata(path)?;
        let size = meta.len() as i64;
        let mtime = meta
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let path_str = path.to_string_lossy();

        let existing: Option<(i64, i64, i64)> = self
            .conn
            .query_row(
                "SELECT id, size, mtime FROM tracks WHERE path = ?1",
                [path_str.as_ref()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((_, s, m)) = existing {
            if s == size && m == mtime {
                return Ok(Outcome::Unchanged);
            }
        }

        let tags = read_tags(path).with_context(|| "read tags")?;
        debug!("{} · {} · {}", tags.album_artist, tags.album, tags.title);

        let tx = self.conn.transaction()?;
        let album_artist_id = upsert_artist(&tx, &tags.album_artist)?;
        let artist_id = if tags.artist == tags.album_artist {
            album_artist_id
        } else {
            upsert_artist(&tx, &tags.artist)?
        };
        let dir = path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let album_id = upsert_album(&tx, album_artist_id, &tags.album, tags.year, &dir)?;

        let track_id: i64 = match existing {
            Some((id, _, _)) => {
                tx.execute(
                    "UPDATE tracks SET album_id=?1, artist_id=?2, title=?3, disc=?4, number=?5, duration_ms=?6,
                       size=?7, mtime=?8, codec=?9, sample_rate=?10, bit_depth=?11, channels=?12, genre=?13
                     WHERE id = ?14",
                    params![album_id, artist_id, tags.title, tags.disc, tags.number, tags.duration_ms, size, mtime,
                        tags.codec, tags.sample_rate, tags.bit_depth, tags.channels, tags.genre, id],
                )?;
                id
            }
            None => {
                tx.execute(
                    "INSERT INTO tracks (album_id, artist_id, title, disc, number, duration_ms, path, size, mtime,
                       codec, sample_rate, bit_depth, channels, genre)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                    params![album_id, artist_id, tags.title, tags.disc, tags.number, tags.duration_ms, path_str.as_ref(),
                        size, mtime, tags.codec, tags.sample_rate, tags.bit_depth, tags.channels, tags.genre],
                )?;
                tx.last_insert_rowid()
            }
        };

        // Art: embedded picture first, then an image file in the folder. Only the
        // first track of an album needs to supply it.
        let has_art: bool = tx.query_row(
            "SELECT art_id IS NOT NULL FROM albums WHERE id = ?1",
            [album_id],
            |r| r.get(0),
        )?;
        if !has_art {
            let found = match tags.picture {
                Some((mime, data)) => Some(db::insert_art(&tx, &mime, &data, "embedded")?),
                None => match folder_art(path.parent().unwrap_or(Path::new("."))) {
                    Some((mime, data)) => Some(db::insert_art(&tx, &mime, &data, "folder")?),
                    None => None,
                },
            };
            if let Some(art_id) = found {
                tx.execute(
                    "UPDATE albums SET art_id = ?1 WHERE id = ?2",
                    params![art_id, album_id],
                )?;
            }
        }

        // Lyrics: embedded tag, then a .lrc or .txt beside the file.
        let local = tags
            .lyrics
            .map(|t| (t, "embedded"))
            .or_else(|| lyrics::sidecar(path).map(|t| (t, "lrc")));
        if let Some((text, source)) = local {
            let synced = lyrics::is_synced(&text);
            tx.execute(
                // Not for lyrics taken away by hand.
                "INSERT INTO lyrics (track_id, synced, text, source)
                 SELECT ?1, ?2, ?3, ?4 WHERE NOT EXISTS
                   (SELECT 1 FROM tracks WHERE id = ?1 AND lyrics_tried = ?5)
                 ON CONFLICT(track_id) DO UPDATE SET synced=excluded.synced, text=excluded.text, source=excluded.source",
                params![track_id, synced as i64, text, source, crate::fetch::LYRICS_REMOVED],
            )?;
        }

        tx.commit()?;
        Ok(if existing.is_some() {
            Outcome::Updated
        } else {
            Outcome::Added
        })
    }
}

enum Outcome {
    Unchanged,
    Added,
    Updated,
}

fn upsert_artist(conn: &Connection, name: &str) -> Result<i64> {
    conn.execute(
        "INSERT OR IGNORE INTO artists (name, sort_name) VALUES (?1, ?2)",
        params![name, sort_name(name)],
    )?;
    Ok(
        conn.query_row("SELECT id FROM artists WHERE name = ?1", [name], |r| {
            r.get(0)
        })?,
    )
}

fn upsert_album(
    conn: &Connection,
    artist_id: i64,
    title: &str,
    year: Option<i64>,
    dir: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO albums (artist_id, title, year, dir) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(artist_id, title, dir) DO UPDATE SET year = COALESCE(albums.year, excluded.year)",
        params![artist_id, title, year, dir],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM albums WHERE artist_id = ?1 AND title = ?2 AND dir = ?3",
        params![artist_id, title, dir],
        |r| r.get(0),
    )?)
}

/// "The Beatles" sorts under B; "Azra" stays "azra".
pub fn sort_name(name: &str) -> String {
    let lower = name.to_lowercase();
    for prefix in ["the ", "a ", "an "] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            return rest.to_string();
        }
    }
    lower
}

fn read_tags(path: &Path) -> Result<FileTags> {
    let tagged = lofty::read_from_path(path)?;
    let props = tagged.properties();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());

    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let folder = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Unknown Album".into());

    let get = |key: ItemKey| -> Option<String> {
        tag.and_then(|t| t.get_string(key))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };

    let title = get(ItemKey::TrackTitle).unwrap_or_else(|| title_from_stem(&stem));
    let artist = get(ItemKey::TrackArtist).unwrap_or_else(|| "Unknown Artist".into());
    let album_artist = get(ItemKey::AlbumArtist).unwrap_or_else(|| artist.clone());
    let album = get(ItemKey::AlbumTitle).unwrap_or(folder);
    let year = get(ItemKey::Year)
        .or_else(|| get(ItemKey::RecordingDate))
        .or_else(|| get(ItemKey::OriginalReleaseDate))
        .and_then(|d| d.get(..4).and_then(|y| y.parse().ok()));
    let disc = tag.and_then(|t| t.disk()).map(i64::from);
    let number = tag
        .and_then(|t| t.track())
        .map(i64::from)
        .or_else(|| number_from_stem(&stem));
    let genre = get(ItemKey::Genre);
    let lyrics = get(ItemKey::Lyrics);

    let picture = tag.and_then(|t| {
        let pics = t.pictures();
        pics.iter()
            .find(|p| p.pic_type() == PictureType::CoverFront)
            .or_else(|| pics.first())
            .map(|p| {
                (
                    p.mime_type()
                        .map(|m| m.as_str().to_owned())
                        .unwrap_or_else(|| "image/jpeg".into()),
                    p.data().to_vec(),
                )
            })
    });

    Ok(FileTags {
        title,
        artist,
        album_artist,
        album,
        year,
        disc,
        number,
        genre,
        duration_ms: props.duration().as_millis() as i64,
        codec: format!("{:?}", tagged.file_type()).to_lowercase(),
        sample_rate: props.sample_rate().map(i64::from),
        bit_depth: props.bit_depth().map(i64::from),
        channels: props.channels().map(i64::from),
        picture,
        lyrics,
    })
}

/// "01 Airbag" -> "Airbag"
fn title_from_stem(stem: &str) -> String {
    let t = stem.trim_start_matches(|c: char| {
        c.is_ascii_digit() || c == '.' || c == '-' || c == '_' || c == ' '
    });
    if t.is_empty() {
        stem.to_string()
    } else {
        t.to_string()
    }
}

/// "01 Airbag" -> 1
fn number_from_stem(stem: &str) -> Option<i64> {
    let digits: String = stem.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn folder_art(dir: &Path) -> Option<(String, Vec<u8>)> {
    const NAMES: &[&str] = &[
        "cover", "folder", "front", "album", "albumart", "Cover", "Folder", "Front",
    ];
    const EXTS: &[(&str, &str)] = &[
        ("jpg", "image/jpeg"),
        ("jpeg", "image/jpeg"),
        ("png", "image/png"),
        ("webp", "image/webp"),
    ];
    for name in NAMES {
        for (ext, mime) in EXTS {
            let p = dir.join(format!("{name}.{ext}"));
            if let Ok(data) = std::fs::read(&p) {
                if !data.is_empty() {
                    return Some(((*mime).to_string(), data));
                }
            }
        }
    }
    None
}
