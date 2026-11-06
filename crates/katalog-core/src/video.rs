//! Films and TV: `Movies/` and `TV-Shows/` next to `Music/`.
//!
//! Names are read the way people save them:
//!
//!   Movies/The Thing (1982)/The Thing (1982).mkv
//!   Movies/Stalker.1979.1080p.BluRay.x264.mkv
//!   TV-Shows/Twin Peaks/Season 1/Twin Peaks - S01E03 - Zen.mkv
//!   TV-Shows/The Wire/The.Wire.S02E05.720p.mkv
//!
//! A show is the first folder under `TV-Shows/`. Lengths and subtitle tracks
//! come from ffprobe when it is installed; subtitle files next to a video
//! (`Film.en.srt`, `Film.srt`, `Subs/English.srt`) count too. A `poster.jpg`
//! (or `folder.jpg`, `cover.jpg`) in the folder is the poster until TMDB
//! finds a better one. Unchanged files (same size and mtime) are skipped.

use crate::db;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};
use tracing::{debug, info};
use walkdir::WalkDir;

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS movies (
  id           INTEGER PRIMARY KEY,
  title        TEXT NOT NULL,
  year         INTEGER,
  path         TEXT NOT NULL UNIQUE,
  size         INTEGER NOT NULL,
  mtime        INTEGER NOT NULL,
  duration_ms  INTEGER NOT NULL DEFAULT 0,
  subs         TEXT NOT NULL DEFAULT '',   -- languages, comma separated; '?' for one with no language
  tmdb_id      INTEGER,
  overview     TEXT,
  released     TEXT,                       -- YYYY-MM-DD
  genres       TEXT,
  poster_id    INTEGER REFERENCES art(id),
  meta_tried   INTEGER NOT NULL DEFAULT 0, -- 1 once TMDB found nothing
  added_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE TABLE IF NOT EXISTS shows (
  id           INTEGER PRIMARY KEY,
  title        TEXT NOT NULL,
  year         INTEGER,
  dir          TEXT NOT NULL UNIQUE,
  tmdb_id      INTEGER,
  overview     TEXT,
  first_aired  TEXT,
  genres       TEXT,
  poster_id    INTEGER REFERENCES art(id),
  meta_tried   INTEGER NOT NULL DEFAULT 0,
  added_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE TABLE IF NOT EXISTS episodes (
  id           INTEGER PRIMARY KEY,
  show_id      INTEGER NOT NULL REFERENCES shows(id) ON DELETE CASCADE,
  season       INTEGER NOT NULL,
  episode      INTEGER NOT NULL,
  title        TEXT,
  path         TEXT NOT NULL UNIQUE,
  size         INTEGER NOT NULL,
  mtime        INTEGER NOT NULL,
  duration_ms  INTEGER NOT NULL DEFAULT 0,
  subs         TEXT NOT NULL DEFAULT '',
  added_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX IF NOT EXISTS episodes_show ON episodes(show_id, season, episode);
"#;

pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "m4v", "avi", "mov", "webm", "mpg", "mpeg", "ts", "wmv",
];
const SUB_EXTENSIONS: &[&str] = &["srt", "ass", "ssa", "vtt", "sub"];
const POSTER_NAMES: &[&str] = &["poster", "folder", "cover"];

fn ext_in(path: &Path, list: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| list.contains(&e.to_ascii_lowercase().as_str()))
}

pub fn is_video_path(path: &Path) -> bool {
    ext_in(path, VIDEO_EXTENSIONS)
}

// ---------------------------------------------------------------- names

/// A film's title and year from its file or folder name.
pub fn parse_movie(name: &str) -> (String, Option<i64>) {
    let words = words_of(name);
    // The year is the last 19xx/20xx that is not the first word ("2001 A Space
    // Odyssey 1968" is 1968; "1917 (2019)" is 2019).
    let year_at = words
        .iter()
        .enumerate()
        .skip(1)
        .rev()
        .find(|(_, w)| year_of(w).is_some())
        .map(|(i, _)| i);
    let end = year_at
        .or_else(|| words.iter().position(|w| is_junk(w)))
        .unwrap_or(words.len());
    let mut title: Vec<&str> = words[..end].iter().map(String::as_str).collect();
    if let Some(p) = title.iter().position(|w| is_junk(w)) {
        title.truncate(p.max(1));
    }
    let year = year_at.and_then(|i| year_of(&words[i]));
    (title.join(" "), year)
}

/// Show name from a folder: "Twin Peaks (1990)" → ("Twin Peaks", 1990).
pub fn parse_show(dir: &str) -> (String, Option<i64>) {
    parse_movie(dir)
}

/// Season, episode and the episode's own title, if the name says.
pub fn parse_episode(name: &str) -> Option<(i64, i64, Option<String>)> {
    let lower = name.to_ascii_lowercase();
    let b = lower.as_bytes();
    let digits = |from: usize| -> (i64, usize) {
        let mut i = from;
        while i < b.len() && b[i].is_ascii_digit() && i - from < 4 {
            i += 1;
        }
        (lower[from..i].parse().unwrap_or(-1), i)
    };
    let mut found = None;
    for i in 0..b.len() {
        // S01E02
        if b[i] == b's'
            && (i == 0 || !b[i - 1].is_ascii_alphabetic())
            && b.get(i + 1).is_some_and(u8::is_ascii_digit)
        {
            let (s, j) = digits(i + 1);
            if b.get(j) == Some(&b'e') && b.get(j + 1).is_some_and(u8::is_ascii_digit) {
                let (e, k) = digits(j + 1);
                found = Some((s, e, k));
                break;
            }
        }
        // 1x02
        if b[i].is_ascii_digit() && (i == 0 || !b[i - 1].is_ascii_alphanumeric()) {
            let (s, j) = digits(i);
            if j - i <= 2 && b.get(j) == Some(&b'x') && b.get(j + 1).is_some_and(u8::is_ascii_digit)
            {
                let (e, k) = digits(j + 1);
                if k - (j + 1) <= 3 && !b.get(k).is_some_and(u8::is_ascii_alphanumeric) {
                    found = Some((s, e, k));
                    break;
                }
            }
        }
    }
    let (s, e, end) = found?;
    if s < 0 || e < 0 {
        return None;
    }
    // A following E03 (S01E02E03) is the same file; skip it.
    let mut rest = &name[end..];
    if rest.len() > 1
        && rest.as_bytes()[0].eq_ignore_ascii_case(&b'e')
        && rest.as_bytes()[1].is_ascii_digit()
    {
        rest = rest[1..].trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let words = words_of(rest);
    let end = words.iter().position(|w| is_junk(w)).unwrap_or(words.len());
    let title = words[..end].join(" ");
    let title = title
        .trim_matches(|c: char| c == '-' || c.is_whitespace())
        .to_string();
    Some((s, e, (!title.is_empty()).then_some(title)))
}

/// A name split into words: dots and underscores are spaces, brackets go.
fn words_of(name: &str) -> Vec<String> {
    let spaced: String = name
        .chars()
        .map(|c| match c {
            '.' | '_' | '(' | ')' | '[' | ']' | '{' | '}' => ' ',
            c => c,
        })
        .collect();
    spaced
        .split_whitespace()
        .skip_while(|w| *w == "-")
        .map(str::to_string)
        .collect()
}

fn year_of(w: &str) -> Option<i64> {
    if w.len() != 4 || !w.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let y: i64 = w.parse().ok()?;
    (1880..=2100).contains(&y).then_some(y)
}

/// Release tags that end a title.
fn is_junk(w: &str) -> bool {
    let w = w.to_ascii_lowercase();
    const TAGS: &[&str] = &[
        "480p",
        "576p",
        "720p",
        "1080p",
        "1080i",
        "2160p",
        "4k",
        "uhd",
        "bluray",
        "blu-ray",
        "brrip",
        "bdrip",
        "webrip",
        "web-dl",
        "webdl",
        "web",
        "hdtv",
        "dvdrip",
        "dvd",
        "remux",
        "x264",
        "x265",
        "h264",
        "h 264",
        "h265",
        "hevc",
        "xvid",
        "10bit",
        "hdr",
        "proper",
        "repack",
        "extended",
        "unrated",
        "remastered",
        "aac",
        "ac3",
        "dts",
        "multi",
        "subbed",
        "dual",
    ];
    TAGS.contains(&w.as_str())
}

// ---------------------------------------------------------------- probing

/// What ffprobe says about a file: its length, and its subtitle languages.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Probe {
    pub duration_ms: i64,
    pub subs: Vec<String>,
}

pub fn probe(path: &Path) -> Probe {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output();
    let mut p = match out {
        Ok(o) if o.status.success() => parse_probe(&o.stdout),
        _ => Probe::default(),
    };
    if p.duration_ms <= 0 {
        // No ffprobe (it comes with ffmpeg, which not every machine has):
        // mpv, which the set needs anyway, can tell the length too.
        p.duration_ms = mpv_duration_ms(path);
    }
    p
}

/// The length of a film or episode as mpv reads it, 0 when it can't.
fn mpv_duration_ms(path: &Path) -> i64 {
    let out = std::process::Command::new("mpv")
        .args([
            "--no-config",
            "--vo=null",
            "--ao=null",
            "--aid=no",
            "--sid=no",
            "--sub-auto=no",
            "--frames=1",
            "--term-playing-msg=KATALOG_DURATION=${=duration}",
        ])
        .arg(path)
        .stdin(std::process::Stdio::null())
        .output();
    let Ok(o) = out else { return 0 };
    parse_mpv_duration(&String::from_utf8_lossy(&o.stdout))
}

fn parse_mpv_duration(said: &str) -> i64 {
    said.lines()
        .find_map(|l| l.trim().strip_prefix("KATALOG_DURATION="))
        .and_then(|d| d.trim().parse::<f64>().ok())
        .map(|secs| (secs * 1000.0) as i64)
        .unwrap_or(0)
}

fn parse_probe(json: &[u8]) -> Probe {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(json) else {
        return Probe::default();
    };
    let secs = v["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let subs = v["streams"]
        .as_array()
        .map(|s| {
            s.iter()
                .filter(|s| s["codec_type"] == "subtitle")
                .map(|s| lang_code(s["tags"]["language"].as_str().unwrap_or("")))
                .collect()
        })
        .unwrap_or_default();
    Probe {
        duration_ms: (secs * 1000.0) as i64,
        subs,
    }
}

/// "eng", "en", "English" → "en". Unknown stays as it is; empty is "?".
pub fn lang_code(s: &str) -> String {
    let l = s.trim().to_ascii_lowercase();
    let code = match l.as_str() {
        "" | "und" => "?",
        "en" | "eng" | "english" => "en",
        "hr" | "hrv" | "scr" | "croatian" | "hrvatski" => "hr",
        "sr" | "srp" | "scc" | "serbian" => "sr",
        "bs" | "bos" | "bosnian" => "bs",
        "sl" | "slv" | "slovenian" => "sl",
        "de" | "deu" | "ger" | "german" => "de",
        "fr" | "fra" | "fre" | "french" => "fr",
        "es" | "spa" | "spanish" => "es",
        "it" | "ita" | "italian" => "it",
        "ru" | "rus" | "russian" => "ru",
        "ja" | "jpn" | "japanese" => "ja",
        other => other,
    };
    code.to_string()
}

/// Subtitle files that belong to a video: next to it with the same stem
/// (`Film.srt`, `Film.en.srt`, `Film.English.forced.srt`), or in a `Subs`
/// folder beside it when the video is alone in its folder.
pub fn sidecar_subs(video: &Path) -> Vec<(PathBuf, String)> {
    let Some(dir) = video.parent() else {
        return vec![];
    };
    let stem = video
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut subs_dirs = Vec::new();
    let mut videos_here = 0;
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            let n = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_lowercase();
            if n == "subs" || n == "subtitles" {
                subs_dirs.push(p);
            }
            continue;
        }
        if is_video_path(&p) {
            videos_here += 1;
        }
        if !ext_in(&p, SUB_EXTENSIONS) {
            continue;
        }
        let s = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if s == stem {
            out.push((p, "?".to_string()));
        } else if let Some(rest) = s.strip_prefix(&format!("{stem}.")) {
            let lang = rest.split('.').next().unwrap_or("");
            out.push((p, lang_code(lang)));
        }
    }
    if videos_here <= 1 {
        for d in subs_dirs {
            for e in WalkDir::new(&d).max_depth(2).into_iter().flatten() {
                let p = e.path();
                if ext_in(p, SUB_EXTENSIONS) {
                    let s = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                    let lang = s.rsplit(['.', '_', ' ']).next().unwrap_or(s);
                    out.push((p.to_path_buf(), lang_code(lang)));
                }
            }
        }
    }
    out.sort();
    out
}

fn subs_field(probe: &Probe, files: &[(PathBuf, String)]) -> String {
    let mut langs: Vec<String> = Vec::new();
    for l in probe.subs.iter().chain(files.iter().map(|(_, l)| l)) {
        if !langs.contains(l) {
            langs.push(l.clone());
        }
    }
    langs.join(",")
}

fn poster_in(dir: &Path) -> Option<(String, Vec<u8>)> {
    for n in POSTER_NAMES {
        for ext in ["jpg", "jpeg", "png", "webp"] {
            let p = dir.join(format!("{n}.{ext}"));
            if let Ok(data) = std::fs::read(&p) {
                let mime = if ext == "png" {
                    "image/png"
                } else if ext == "webp" {
                    "image/webp"
                } else {
                    "image/jpeg"
                };
                return Some((mime.to_string(), data));
            }
        }
    }
    None
}

// ---------------------------------------------------------------- scanning

#[derive(Debug, Default, Clone, Serialize)]
pub struct VideoReport {
    pub movies: usize,
    pub episodes: usize,
    pub added: usize,
    pub removed: usize,
    pub seconds: f64,
}

/// Brings the database in line with `<library>/Movies` and `<library>/TV-Shows`.
/// Either folder may be missing.
pub fn scan(conn: &mut Connection, library: &Path) -> Result<VideoReport> {
    scan_with(conn, library, &probe)
}

pub fn scan_with(
    conn: &mut Connection,
    library: &Path,
    probe: &dyn Fn(&Path) -> Probe,
) -> Result<VideoReport> {
    let started = Instant::now();
    let mut r = VideoReport::default();
    let tx = conn.transaction()?;
    scan_movies(&tx, &library.join("Movies"), probe, &mut r)?;
    scan_shows(&tx, &library.join("TV-Shows"), probe, &mut r)?;
    tx.commit()?;
    r.seconds = started.elapsed().as_secs_f64();
    if r.added + r.removed > 0 {
        info!(
            "video: {} films, {} episodes ({} new, {} gone)",
            r.movies, r.episodes, r.added, r.removed
        );
    }
    Ok(r)
}

fn stat(p: &Path) -> Option<(i64, i64)> {
    let m = std::fs::metadata(p).ok()?;
    let mtime = m
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    Some((m.len() as i64, mtime))
}

/// Folders that hold extras, as Plex, Jellyfin and Kodi name them.
const EXTRA_DIRS: &[&str] = &[
    "extras",
    "extra",
    "featurettes",
    "featurette",
    "behind the scenes",
    "behindthescenes",
    "deleted scenes",
    "deletedscenes",
    "interviews",
    "interview",
    "scenes",
    "shorts",
    "trailers",
    "trailer",
    "other",
    "others",
    "sample",
    "samples",
    "bonus",
    "bonus features",
    "making of",
];
/// Endings that mark a file as an extra: `Alien-trailer.mkv`.
const EXTRA_SUFFIXES: &[&str] = &[
    "sample",
    "trailer",
    "featurette",
    "behindthescenes",
    "deleted",
    "deletedscene",
    "interview",
    "scene",
    "short",
    "other",
    "extra",
    "teaser",
];

fn is_extra(root: &Path, p: &Path) -> bool {
    let rel = p.strip_prefix(root).unwrap_or(p);
    let in_extras_dir = rel.parent().is_some_and(|d| {
        d.components().any(|c| {
            let c = c.as_os_str().to_string_lossy().to_lowercase();
            EXTRA_DIRS.contains(&c.trim())
        })
    });
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let marked = stem == "sample"
        || stem == "trailer"
        || EXTRA_SUFFIXES.iter().any(|x| {
            stem.ends_with(&format!("-{x}"))
                || stem.ends_with(&format!(".{x}"))
                || stem.ends_with(&format!("_{x}"))
        });
    in_extras_dir || marked
}

/// The films under `Movies/`. A folder that holds a film holds that film:
/// any video in a folder below it (`Featurettes/`, `Bonus/`, whatever it is
/// called) is an extra. Collections still work, since
/// `Alien Collection/Alien (1979)/Alien (1979).mkv` has no film above it.
fn films_under(root: &Path) -> Vec<PathBuf> {
    let all = videos_under(root);
    let holds_film: HashSet<&Path> = all.iter().filter_map(|p| p.parent()).collect();
    all.iter()
        .filter(|p| {
            let mut d = p.parent().and_then(Path::parent);
            while let Some(a) = d {
                if a == root || !a.starts_with(root) {
                    return true;
                }
                if holds_film.contains(a) {
                    return false;
                }
                d = a.parent();
            }
            true
        })
        .cloned()
        .collect()
}

/// The episodes of a show: files named S01E02 (or 1x02), or anything in a
/// `Season 1` or `Specials` folder. The rest is extras.
fn episodes_under(show: &Path) -> Vec<PathBuf> {
    videos_under(show)
        .into_iter()
        .filter(|p| {
            let stem = p.file_stem().unwrap_or_default().to_string_lossy();
            let season_dir = p
                .parent()
                .filter(|d| *d != show)
                .and_then(|d| d.file_name())
                .map(|n| n.to_string_lossy().to_lowercase())
                .is_some_and(|n| n.starts_with("season") || n == "specials");
            parse_episode(&stem).is_some() || season_dir
        })
        .collect()
}

/// Every video under `root` that is not an extra.
fn videos_under(root: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file() && is_video_path(e.path()))
        .map(|e| e.into_path())
        .filter(|p| !is_extra(root, p))
        .collect();
    v.sort();
    v
}

fn scan_movies(
    conn: &Connection,
    root: &Path,
    probe: &dyn Fn(&Path) -> Probe,
    r: &mut VideoReport,
) -> Result<()> {
    let mut seen = HashSet::new();
    if root.is_dir() {
        let films = films_under(root);
        for path in films.iter().cloned() {
            let key = path.to_string_lossy().to_string();
            seen.insert(key.clone());
            let Some((size, mtime)) = stat(&path) else {
                continue;
            };
            let known: Option<(i64, i64, i64, bool)> = conn
                .query_row(
                    "SELECT id, size, mtime, duration_ms > 0 FROM movies WHERE path = ?1",
                    [&key],
                    |x| Ok((x.get(0)?, x.get(1)?, x.get(2)?, x.get(3)?)),
                )
                .optional()?;
            r.movies += 1;
            // Unchanged and with a length: nothing to do. One without a length
            // (probed before mpv could stand in for ffprobe) is tried again.
            if known.is_some_and(|(_, s, m, timed)| s == size && m == mtime && timed) {
                continue;
            }
            // A film alone in its folder takes the folder's name, which is
            // usually the tidier one.
            let dir = path.parent().unwrap_or(root);
            let alone =
                dir != root && films.iter().filter(|f| f.parent() == Some(dir)).count() == 1;
            let name = if alone {
                dir.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            } else {
                path.file_stem()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            };
            let (title, year) = parse_movie(&name);
            let p = probe(&path);
            let subs = subs_field(&p, &sidecar_subs(&path));
            let poster = if alone { poster_in(dir) } else { None };
            let poster_id = match poster {
                Some((mime, data)) => Some(db::insert_art(conn, &mime, &data, "folder")?),
                None => None,
            };
            match known {
                Some((id, ..)) => {
                    conn.execute(
                        "UPDATE movies SET size = ?2, mtime = ?3, duration_ms = ?4, subs = ?5,
                           poster_id = COALESCE(poster_id, ?6) WHERE id = ?1",
                        params![id, size, mtime, p.duration_ms, subs, poster_id],
                    )?;
                }
                None => {
                    conn.execute(
                        "INSERT INTO movies (title, year, path, size, mtime, duration_ms, subs, poster_id)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        params![title, year, key, size, mtime, p.duration_ms, subs, poster_id],
                    )?;
                    r.added += 1;
                    debug!("film: {title} ({year:?})");
                }
            }
        }
    }
    let gone: Vec<(i64, String)> = {
        let mut st = conn.prepare("SELECT id, path FROM movies")?;
        let rows = st.query_map([], |x| Ok((x.get(0)?, x.get(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter(|(_, p)| !seen.contains(p))
            .collect()
    };
    for (id, _) in gone {
        conn.execute("DELETE FROM movies WHERE id = ?1", [id])?;
        r.removed += 1;
    }
    Ok(())
}

fn scan_shows(
    conn: &Connection,
    root: &Path,
    probe: &dyn Fn(&Path) -> Probe,
    r: &mut VideoReport,
) -> Result<()> {
    let mut seen = HashSet::new();
    let mut seen_shows = HashSet::new();
    if root.is_dir() {
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for dir in dirs {
            let dkey = dir.to_string_lossy().to_string();
            let files = episodes_under(&dir);
            if files.is_empty() {
                continue;
            }
            seen_shows.insert(dkey.clone());
            let show_id: i64 = match conn
                .query_row("SELECT id FROM shows WHERE dir = ?1", [&dkey], |x| x.get(0))
                .optional()?
            {
                Some(id) => id,
                None => {
                    let (title, year) =
                        parse_show(&dir.file_name().unwrap_or_default().to_string_lossy());
                    conn.execute(
                        "INSERT INTO shows (title, year, dir) VALUES (?1, ?2, ?3)",
                        params![title, year, dkey],
                    )?;
                    conn.last_insert_rowid()
                }
            };
            if let Some((mime, data)) = poster_in(&dir) {
                let art = db::insert_art(conn, &mime, &data, "folder")?;
                conn.execute(
                    "UPDATE shows SET poster_id = COALESCE(poster_id, ?2) WHERE id = ?1",
                    params![show_id, art],
                )?;
            }
            for (n, path) in files.iter().enumerate() {
                let key = path.to_string_lossy().to_string();
                seen.insert(key.clone());
                let Some((size, mtime)) = stat(path) else {
                    continue;
                };
                let known: Option<(i64, i64, i64, bool)> = conn
                    .query_row(
                        "SELECT id, size, mtime, duration_ms > 0 FROM episodes WHERE path = ?1",
                        [&key],
                        |x| Ok((x.get(0)?, x.get(1)?, x.get(2)?, x.get(3)?)),
                    )
                    .optional()?;
                r.episodes += 1;
                // Unchanged and with a length: nothing to do. One without a length
                // (probed before mpv could stand in for ffprobe) is tried again.
                if known.is_some_and(|(_, s, m, timed)| s == size && m == mtime && timed) {
                    continue;
                }
                let stem = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                // No SxxEyy: the season from a "Season 2" folder, the episode from
                // the file's place in it.
                let (season, episode, title) = parse_episode(&stem).unwrap_or_else(|| {
                    let season = path
                        .parent()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().to_lowercase())
                        .and_then(|n| {
                            n.strip_prefix("season")
                                .map(|s| s.trim().parse().unwrap_or(1))
                        })
                        .unwrap_or(1);
                    (season, n as i64 + 1, Some(words_of(&stem).join(" ")))
                });
                let p = probe(path);
                let subs = subs_field(&p, &sidecar_subs(path));
                match known {
                    Some((id, ..)) => {
                        conn.execute(
                            "UPDATE episodes SET size = ?2, mtime = ?3, duration_ms = ?4, subs = ?5 WHERE id = ?1",
                            params![id, size, mtime, p.duration_ms, subs],
                        )?;
                    }
                    None => {
                        conn.execute(
                            "INSERT INTO episodes (show_id, season, episode, title, path, size, mtime, duration_ms, subs)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                            params![show_id, season, episode, title, key, size, mtime, p.duration_ms, subs],
                        )?;
                        r.added += 1;
                    }
                }
            }
        }
    }
    let gone: Vec<i64> = {
        let mut st = conn.prepare("SELECT id, path FROM episodes")?;
        let rows = st.query_map([], |x| Ok((x.get::<_, i64>(0)?, x.get::<_, String>(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter(|(_, p)| !seen.contains(p))
            .map(|(id, _)| id)
            .collect()
    };
    for id in gone {
        conn.execute("DELETE FROM episodes WHERE id = ?1", [id])?;
        r.removed += 1;
    }
    let empty: Vec<(i64, String)> = {
        let mut st = conn.prepare("SELECT id, dir FROM shows")?;
        let rows = st.query_map([], |x| Ok((x.get(0)?, x.get(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (id, dir) in empty {
        if !seen_shows.contains(&dir) {
            conn.execute("DELETE FROM shows WHERE id = ?1", [id])?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- reading

#[derive(Debug, Clone, Serialize)]
pub struct Movie {
    pub id: i64,
    pub title: String,
    pub year: Option<i64>,
    pub path: String,
    pub duration_ms: i64,
    /// Subtitle languages, "en" first if there is one.
    pub subs: Vec<String>,
    pub overview: Option<String>,
    pub released: Option<String>,
    pub genres: Vec<String>,
    pub poster_id: Option<i64>,
    pub tmdb_id: Option<i64>,
    pub added_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Show {
    pub id: i64,
    pub title: String,
    pub year: Option<i64>,
    pub overview: Option<String>,
    pub first_aired: Option<String>,
    pub genres: Vec<String>,
    pub poster_id: Option<i64>,
    pub tmdb_id: Option<i64>,
    pub seasons: i64,
    pub episode_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Episode {
    pub id: i64,
    pub show_id: i64,
    pub season: i64,
    pub episode: i64,
    pub title: Option<String>,
    pub path: String,
    pub duration_ms: i64,
    pub subs: Vec<String>,
}

fn list(s: Option<String>) -> Vec<String> {
    let mut v: Vec<String> = s
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(str::to_string)
        .collect();
    if let Some(i) = v.iter().position(|l| l == "en") {
        let en = v.remove(i);
        v.insert(0, en);
    }
    v
}

const MOVIE_SQL: &str =
    "SELECT id, title, year, path, duration_ms, subs, overview, released, genres,
    poster_id, tmdb_id, added_at FROM movies";

fn movie_from(r: &rusqlite::Row) -> rusqlite::Result<Movie> {
    Ok(Movie {
        id: r.get(0)?,
        title: r.get(1)?,
        year: r.get(2)?,
        path: r.get(3)?,
        duration_ms: r.get(4)?,
        subs: list(r.get(5)?),
        overview: r.get(6)?,
        released: r.get(7)?,
        genres: list(r.get(8)?),
        poster_id: r.get(9)?,
        tmdb_id: r.get(10)?,
        added_at: r.get(11)?,
    })
}

/// Films, by title, optionally only those matching `q` (title, year or genre).
pub fn movies(conn: &Connection, q: Option<&str>) -> Result<Vec<Movie>> {
    let like = q.map(|q| format!("%{}%", q.trim()));
    let mut st = conn.prepare(&format!(
        "{MOVIE_SQL} WHERE ?1 IS NULL OR title LIKE ?1 OR CAST(year AS TEXT) LIKE ?1 OR genres LIKE ?1
         ORDER BY title COLLATE NOCASE, year"
    ))?;
    let rows = st.query_map([like], movie_from)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn movie(conn: &Connection, id: i64) -> Result<Option<Movie>> {
    Ok(conn
        .query_row(&format!("{MOVIE_SQL} WHERE id = ?1"), [id], movie_from)
        .optional()?)
}

const SHOW_SQL: &str =
    "SELECT s.id, s.title, s.year, s.overview, s.first_aired, s.genres, s.poster_id, s.tmdb_id,
    (SELECT COUNT(DISTINCT season) FROM episodes e WHERE e.show_id = s.id),
    (SELECT COUNT(*) FROM episodes e WHERE e.show_id = s.id)
    FROM shows s";

fn show_from(r: &rusqlite::Row) -> rusqlite::Result<Show> {
    Ok(Show {
        id: r.get(0)?,
        title: r.get(1)?,
        year: r.get(2)?,
        overview: r.get(3)?,
        first_aired: r.get(4)?,
        genres: list(r.get(5)?),
        poster_id: r.get(6)?,
        tmdb_id: r.get(7)?,
        seasons: r.get(8)?,
        episode_count: r.get(9)?,
    })
}

pub fn shows(conn: &Connection, q: Option<&str>) -> Result<Vec<Show>> {
    let like = q.map(|q| format!("%{}%", q.trim()));
    let mut st = conn.prepare(&format!(
        "{SHOW_SQL} WHERE ?1 IS NULL OR s.title LIKE ?1 OR s.genres LIKE ?1 ORDER BY s.title COLLATE NOCASE"
    ))?;
    let rows = st.query_map([like], show_from)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn show(conn: &Connection, id: i64) -> Result<Option<Show>> {
    Ok(conn
        .query_row(&format!("{SHOW_SQL} WHERE s.id = ?1"), [id], show_from)
        .optional()?)
}

const EPISODE_SQL: &str =
    "SELECT id, show_id, season, episode, title, path, duration_ms, subs FROM episodes";

fn episode_from(r: &rusqlite::Row) -> rusqlite::Result<Episode> {
    Ok(Episode {
        id: r.get(0)?,
        show_id: r.get(1)?,
        season: r.get(2)?,
        episode: r.get(3)?,
        title: r.get(4)?,
        path: r.get(5)?,
        duration_ms: r.get(6)?,
        subs: list(r.get(7)?),
    })
}

pub fn episodes(conn: &Connection, show_id: i64) -> Result<Vec<Episode>> {
    let mut st = conn.prepare(&format!(
        "{EPISODE_SQL} WHERE show_id = ?1 ORDER BY season, episode, path"
    ))?;
    let rows = st.query_map([show_id], episode_from)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn episode(conn: &Connection, id: i64) -> Result<Option<Episode>> {
    Ok(conn
        .query_row(&format!("{EPISODE_SQL} WHERE id = ?1"), [id], episode_from)
        .optional()?)
}

// ---------------------------------------------------------------- watching

/// Everything a player needs to show a film or an episode: the file, its
/// subtitle files, and which subtitle track to turn on.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Watch {
    pub title: String,
    pub path: String,
    pub duration_ms: i64,
    /// Subtitle files beside the video, in the order they are to be loaded.
    pub sub_files: Vec<String>,
    /// mpv's `--sid`: the English track's number, or "no".
    pub sid: String,
}

/// mpv numbers subtitle tracks from 1: the file's own first, then each
/// `--sub-file` in the order given. English is turned on if there is one: a
/// track the file calls English, then a file named so (`Film.en.srt`), then
/// a file with no language at all (`Film.srt`), which is nearly always the
/// one put there to be read. Otherwise there are no subtitles.
pub fn pick_sid(embedded: &[String], files: &[String]) -> String {
    if let Some(i) = embedded.iter().position(|l| l == "en") {
        return (i + 1).to_string();
    }
    let n = embedded.len();
    if let Some(i) = files.iter().position(|l| l == "en") {
        return (n + i + 1).to_string();
    }
    if let Some(i) = files.iter().position(|l| l == "?") {
        return (n + i + 1).to_string();
    }
    "no".into()
}

pub fn watch(title: String, path: &str, duration_ms: i64) -> Watch {
    let p = Path::new(path);
    let embedded = probe(p).subs;
    let files = sidecar_subs(p);
    let langs: Vec<String> = files.iter().map(|(_, l)| l.clone()).collect();
    Watch {
        title,
        path: path.to_string(),
        duration_ms,
        sid: pick_sid(&embedded, &langs),
        sub_files: files
            .into_iter()
            .map(|(f, _)| f.to_string_lossy().to_string())
            .collect(),
    }
}

/// The title, path and length of a film or an episode, to
/// hand to `watch` once the database is let go: probing takes a moment.
pub fn watch_target(
    conn: &Connection,
    episode_id: Option<i64>,
    movie_id: Option<i64>,
) -> Result<Option<(String, String, i64)>> {
    if let Some(id) = movie_id {
        return Ok(movie(conn, id)?.map(|m| {
            let title = match m.year {
                Some(y) => format!("{} ({y})", m.title),
                None => m.title.clone(),
            };
            (title, m.path, m.duration_ms)
        }));
    }
    let Some(e) = episode_id
        .map(|id| episode(conn, id))
        .transpose()?
        .flatten()
    else {
        return Ok(None);
    };
    let show = show(conn, e.show_id)?.map(|s| s.title).unwrap_or_default();
    let mut title = format!("{show} · S{:02}E{:02}", e.season, e.episode);
    if let Some(t) = &e.title {
        title.push_str(&format!(" · {t}"));
    }
    Ok(Some((title, e.path, e.duration_ms)))
}

// ---------------------------------------------------------------- TMDB

/// A film or show still without TMDB details.
#[derive(Debug, Clone)]
pub struct Pending {
    pub id: i64,
    pub title: String,
    pub year: Option<i64>,
}

pub fn pending_movies(conn: &Connection, limit: usize) -> Result<Vec<Pending>> {
    pending(conn, "movies", limit)
}

pub fn pending_shows(conn: &Connection, limit: usize) -> Result<Vec<Pending>> {
    pending(conn, "shows", limit)
}

fn pending(conn: &Connection, table: &str, limit: usize) -> Result<Vec<Pending>> {
    let mut st = conn.prepare(&format!(
        "SELECT id, title, year FROM {table} WHERE tmdb_id IS NULL AND meta_tried = 0 ORDER BY id LIMIT ?1"
    ))?;
    let rows = st.query_map([limit as i64], |r| {
        Ok(Pending {
            id: r.get(0)?,
            title: r.get(1)?,
            year: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn reset_meta_tried(conn: &Connection) -> Result<usize> {
    Ok(conn.execute("UPDATE movies SET meta_tried = 0", [])?
        + conn.execute("UPDATE shows SET meta_tried = 0", [])?)
}

pub fn mark_missing(conn: &Connection, kind: Kind, id: i64) -> Result<()> {
    conn.execute(
        &format!("UPDATE {} SET meta_tried = 1 WHERE id = ?1", kind.table()),
        [id],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Movie,
    Show,
}

impl Kind {
    fn table(self) -> &'static str {
        match self {
            Kind::Movie => "movies",
            Kind::Show => "shows",
        }
    }
    fn path(self) -> &'static str {
        match self {
            Kind::Movie => "movie",
            Kind::Show => "tv",
        }
    }
}

/// What TMDB knows about a title.
#[derive(Debug, Clone, PartialEq)]
pub struct Details {
    pub tmdb_id: i64,
    pub overview: Option<String>,
    pub date: Option<String>,
    pub genres: Vec<String>,
    pub poster_path: Option<String>,
    pub runtime_min: Option<i64>,
}

/// Stores TMDB's details; a poster from TMDB replaces one from the folder.
pub fn store_details(
    conn: &Connection,
    kind: Kind,
    id: i64,
    d: &Details,
    poster: Option<(String, Vec<u8>)>,
) -> Result<()> {
    let poster_id = match poster {
        Some((mime, data)) => Some(db::insert_art(conn, &mime, &data, "tmdb")?),
        None => None,
    };
    let date_col = if kind == Kind::Movie {
        "released"
    } else {
        "first_aired"
    };
    conn.execute(
        &format!(
            "UPDATE {} SET tmdb_id = ?2, overview = ?3, {date_col} = ?4, genres = ?5,
               poster_id = COALESCE(?6, poster_id) WHERE id = ?1",
            kind.table()
        ),
        params![
            id,
            d.tmdb_id,
            d.overview,
            d.date,
            d.genres.join(","),
            poster_id
        ],
    )?;
    Ok(())
}

pub struct Tmdb {
    http: reqwest::Client,
    key: String,
}

const TMDB: &str = "https://api.themoviedb.org/3";
const TMDB_IMG: &str = "https://image.tmdb.org/t/p/w500";

impl Tmdb {
    /// `key` is a v3 API key or a v4 read access token, as TMDB hands them out.
    pub fn new(key: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(crate::fetch::USER_AGENT)
            .timeout(std::time::Duration::from_secs(20))
            .build()?;
        Ok(Self {
            http,
            key: key.trim().to_string(),
        })
    }

    async fn get(&self, path: &str, query: &[(&str, String)]) -> Result<serde_json::Value> {
        let mut req = self.http.get(format!("{TMDB}{path}")).query(query);
        // A read token is a long JWT; a v3 key is 32 hex characters.
        req = if self.key.len() > 40 {
            req.bearer_auth(&self.key)
        } else {
            req.query(&[("api_key", &self.key)])
        };
        let res = req.send().await?;
        if res.status() == reqwest::StatusCode::UNAUTHORIZED {
            anyhow::bail!("TMDB refused the key (401): check TMDB_KEY");
        }
        Ok(res.error_for_status()?.json().await?)
    }

    /// The best match for a title, with its details, or None when TMDB has nothing.
    pub async fn find(
        &self,
        kind: Kind,
        title: &str,
        year: Option<i64>,
    ) -> Result<Option<Details>> {
        let mut q = vec![
            ("query", title.to_string()),
            ("include_adult", "false".into()),
        ];
        if let Some(y) = year {
            q.push((
                if kind == Kind::Movie {
                    "year"
                } else {
                    "first_air_date_year"
                },
                y.to_string(),
            ));
        }
        let mut v = self.get(&format!("/search/{}", kind.path()), &q).await?;
        // A wrong year in a file name should not hide the film.
        if year.is_some() && v["results"].as_array().is_none_or(|a| a.is_empty()) {
            v = self
                .get(&format!("/search/{}", kind.path()), &q[..2])
                .await?;
        }
        let Some(id) = best_result(&v, title, year) else {
            return Ok(None);
        };
        let d = self.get(&format!("/{}/{id}", kind.path()), &[]).await?;
        Ok(Some(details_from(kind, &d)))
    }

    pub async fn poster(&self, path: &str) -> Result<Option<(String, Vec<u8>)>> {
        let res = self.http.get(format!("{TMDB_IMG}{path}")).send().await?;
        if !res.status().is_success() {
            return Ok(None);
        }
        let mime = res
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/jpeg")
            .to_string();
        Ok(Some((mime, res.bytes().await?.to_vec())))
    }
}

/// The search result to trust: an exact title (and year) first, then TMDB's own first.
fn best_result(v: &serde_json::Value, title: &str, year: Option<i64>) -> Option<i64> {
    let results = v["results"].as_array()?;
    let norm = |s: &str| {
        s.to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
    };
    let want = norm(title);
    let year_of = |r: &serde_json::Value| -> Option<i64> {
        r["release_date"]
            .as_str()
            .or(r["first_air_date"].as_str())
            .and_then(|d| d.get(..4))
            .and_then(|y| y.parse().ok())
    };
    let name = |r: &serde_json::Value| -> Vec<String> {
        ["title", "name", "original_title", "original_name"]
            .iter()
            .filter_map(|k| r[*k].as_str())
            .map(norm)
            .collect()
    };
    results
        .iter()
        .find(|r| name(r).contains(&want) && (year.is_none() || year_of(r) == year))
        .or_else(|| results.iter().find(|r| name(r).contains(&want)))
        .or_else(|| results.first())
        .and_then(|r| r["id"].as_i64())
}

fn details_from(kind: Kind, d: &serde_json::Value) -> Details {
    let s = |k: &str| d[k].as_str().filter(|s| !s.is_empty()).map(str::to_string);
    Details {
        tmdb_id: d["id"].as_i64().unwrap_or(0),
        overview: s("overview"),
        date: if kind == Kind::Movie {
            s("release_date")
        } else {
            s("first_air_date")
        },
        genres: d["genres"]
            .as_array()
            .map(|g| {
                g.iter()
                    .filter_map(|g| g["name"].as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        poster_path: s("poster_path"),
        runtime_min: d["runtime"].as_i64().filter(|m| *m > 0),
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn mpv_tells_the_length() {
        let said = " (+) Video --vid=1\nKATALOG_DURATION=5.003000\nExiting... (End of file)\n";
        assert_eq!(parse_mpv_duration(said), 5003);
        assert_eq!(parse_mpv_duration("KATALOG_DURATION=\n"), 0);
        assert_eq!(parse_mpv_duration(""), 0);
    }

    use super::*;

    #[test]
    fn film_names() {
        assert_eq!(
            parse_movie("The Thing (1982)"),
            ("The Thing".into(), Some(1982))
        );
        assert_eq!(
            parse_movie("Stalker.1979.1080p.BluRay.x264"),
            ("Stalker".into(), Some(1979))
        );
        assert_eq!(parse_movie("1917 (2019)"), ("1917".into(), Some(2019)));
        assert_eq!(
            parse_movie("2001.A.Space.Odyssey.1968.REMASTERED.2160p"),
            ("2001 A Space Odyssey".into(), Some(1968))
        );
        assert_eq!(
            parse_movie("Blade Runner 2049 (2017)"),
            ("Blade Runner 2049".into(), Some(2017))
        );
        assert_eq!(parse_movie("Paris, Texas"), ("Paris, Texas".into(), None));
        assert_eq!(parse_movie("Alien.1080p.WEB-DL"), ("Alien".into(), None));
        assert_eq!(
            parse_movie("Ko to tamo peva [1980]"),
            ("Ko to tamo peva".into(), Some(1980))
        );
    }

    #[test]
    fn episode_names() {
        assert_eq!(
            parse_episode("Twin Peaks - S01E03 - Zen"),
            Some((1, 3, Some("Zen".into())))
        );
        assert_eq!(
            parse_episode("The.Wire.S02E05.720p.HDTV"),
            Some((2, 5, None))
        );
        assert_eq!(
            parse_episode("Show 3x07 The One"),
            Some((3, 7, Some("The One".into())))
        );
        assert_eq!(
            parse_episode("Show.S01E01E02.Pilot"),
            Some((1, 1, Some("Pilot".into())))
        );
        assert_eq!(parse_episode("Episode one"), None);
        // A resolution is not an episode.
        assert_eq!(parse_episode("Film 1920x1080"), None);
    }

    #[test]
    fn ffprobe_output() {
        let p = parse_probe(
            br#"{"streams":[{"codec_type":"video"},{"codec_type":"subtitle","tags":{"language":"hrv"}},
                {"codec_type":"subtitle","tags":{"language":"eng"}},{"codec_type":"subtitle"}],
                "format":{"duration":"5412.320000"}}"#,
        );
        assert_eq!(p.duration_ms, 5_412_320);
        assert_eq!(p.subs, ["hr", "en", "?"]);
        assert_eq!(list(Some("hr,en,?".into())), ["en", "hr", "?"]);
    }

    #[test]
    fn english_subtitles_are_picked_when_there_are_any() {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(pick_sid(&v(&["hr", "en"]), &v(&["en"])), "2");
        assert_eq!(pick_sid(&v(&["hr"]), &v(&["de", "en"])), "3");
        assert_eq!(pick_sid(&v(&[]), &v(&["?"])), "1");
        assert_eq!(pick_sid(&v(&["hr", "?"]), &v(&["de"])), "no");
        assert_eq!(pick_sid(&v(&[]), &v(&[])), "no");
    }

    #[test]
    fn tmdb_picks_the_exact_title_and_year() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"results":[{"id":1,"title":"The Thing Called Love","release_date":"1993-08-27"},
                {"id":2,"title":"The Thing","release_date":"2011-10-12"},
                {"id":3,"title":"The Thing","release_date":"1982-06-25"}]}"#,
        )
        .unwrap();
        assert_eq!(best_result(&v, "The Thing", Some(1982)), Some(3));
        assert_eq!(best_result(&v, "The Thing", None), Some(2));
        assert_eq!(best_result(&v, "Unrelated", None), Some(1));
        let d = details_from(
            Kind::Movie,
            &serde_json::json!({"id": 3, "overview": "Antarctica.", "release_date": "1982-06-25",
                "genres": [{"name": "Horror"}, {"name": "Science Fiction"}], "poster_path": "/t.jpg", "runtime": 109}),
        );
        assert_eq!(d.genres, ["Horror", "Science Fiction"]);
        assert_eq!(d.runtime_min, Some(109));
    }

    #[test]
    fn a_library_is_scanned_and_kept_in_line() {
        let root = std::env::temp_dir().join(format!("katalog-video-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mk = |p: &str| {
            let p = root.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
        };
        mk("Movies/The Thing (1982)/The Thing (1982).mkv");
        mk("Movies/The Thing (1982)/The Thing (1982).en.srt");
        mk("Movies/The Thing (1982)/poster.jpg");
        mk("Movies/The Thing (1982)/Extras/Making of.mkv");
        mk("Movies/The Thing (1982)/Behind The Scenes/Crew.mkv");
        mk("Movies/The Thing (1982)/Bonus Disc/Commentary.mkv");
        mk("Movies/The Thing (1982)/The Thing-trailer.mkv");
        mk("Movies/Alien Collection/Alien (1979)/Alien (1979).mkv");
        mk("Movies/Alien Collection/Alien (1979)/Featurettes/Chestburster.mkv");
        mk("TV-Shows/Twin Peaks (1990)/Extras/Lynch interview.mkv");
        mk("TV-Shows/Twin Peaks (1990)/Season 1/Deleted Scenes/Scene 1.mkv");
        mk("TV-Shows/Twin Peaks (1990)/Promo reel.mkv");
        mk("Movies/Stalker.1979.1080p.BluRay.x264.mkv");
        mk("TV-Shows/Twin Peaks (1990)/Season 1/Twin Peaks - S01E01 - Pilot.mkv");
        mk("TV-Shows/Twin Peaks (1990)/Season 1/Twin Peaks - S01E02.mkv");
        mk("TV-Shows/Twin Peaks (1990)/Season 2/Twin Peaks - S02E01.mkv");
        let mut conn = db::open(&root.join("k.db")).unwrap();
        let fake = |_: &Path| Probe {
            duration_ms: 60_000,
            subs: vec!["hr".into()],
        };
        let r = scan_with(&mut conn, &root, &fake).unwrap();
        assert_eq!((r.movies, r.episodes, r.added), (3, 3, 6));
        let m = movies(&conn, None).unwrap();
        assert_eq!(
            m.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(),
            ["Alien", "Stalker", "The Thing"]
        );
        assert_eq!(m[2].subs, ["en", "hr"]);
        assert!(m[2].poster_id.is_some() && m[1].poster_id.is_none());
        let s = shows(&conn, None).unwrap();
        assert_eq!(
            (
                s[0].title.as_str(),
                s[0].year,
                s[0].seasons,
                s[0].episode_count
            ),
            ("Twin Peaks", Some(1990), 2, 3)
        );
        let e = episodes(&conn, s[0].id).unwrap();
        assert_eq!(e[0].title.as_deref(), Some("Pilot"));
        // Unchanged: nothing new. A removed episode goes.
        assert_eq!(scan_with(&mut conn, &root, &fake).unwrap().added, 0);
        std::fs::remove_file(
            root.join("TV-Shows/Twin Peaks (1990)/Season 2/Twin Peaks - S02E01.mkv"),
        )
        .unwrap();
        assert_eq!(scan_with(&mut conn, &root, &fake).unwrap().removed, 1);
        assert_eq!(shows(&conn, None).unwrap()[0].seasons, 1);
        let _ = std::fs::remove_dir_all(&root);
    }
}
