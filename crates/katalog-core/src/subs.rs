//! Subtitles from OpenSubtitles.com for a film or episode that has none in
//! the language wanted. It needs an API key from a (free) opensubtitles.com
//! account; downloads are limited per day by the account.
//!
//! A hit is saved beside the video as `Film.<lang>.srt`, where the scan and
//! mpv find it like any other subtitle file.

use crate::fetch::USER_AGENT;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

const API: &str = "https://api.opensubtitles.com/api/v1";

/// What to look for.
pub struct Wanted<'a> {
    pub title: &'a str,
    pub year: Option<i64>,
    /// The film's TMDB id, or for an episode its show's.
    pub tmdb_id: Option<i64>,
    /// Season and episode, for an episode.
    pub episode: Option<(i64, i64)>,
    pub path: &'a Path,
}

/// One subtitle file to pick from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubHit {
    pub file_id: i64,
    pub language: String,
    /// The release it was made for, which says how well it will be timed.
    pub release: String,
    pub downloads: i64,
    /// Made for this very file (same hash), so in sync.
    pub exact: bool,
    pub hearing_impaired: bool,
}

pub struct OpenSubtitles {
    http: reqwest::Client,
    key: String,
}

#[derive(Deserialize)]
struct SearchReply {
    #[serde(default)]
    data: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    attributes: Attributes,
}

#[derive(Deserialize)]
struct Attributes {
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    release: Option<String>,
    #[serde(default)]
    download_count: i64,
    #[serde(default)]
    hearing_impaired: bool,
    #[serde(default)]
    moviehash_match: bool,
    #[serde(default)]
    files: Vec<File>,
}

#[derive(Deserialize)]
struct File {
    file_id: i64,
}

#[derive(Deserialize)]
struct DownloadReply {
    link: Option<String>,
    message: Option<String>,
}

impl OpenSubtitles {
    pub fn new(key: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(20))
            .build()?;
        Ok(Self {
            http,
            key: key.trim().to_string(),
        })
    }

    /// Subtitles for `w` in `langs` ("en,hr"), the best first: made for this
    /// file, then the most downloaded.
    pub async fn search(&self, w: &Wanted<'_>, langs: &str) -> Result<Vec<SubHit>> {
        let query = search_query(w, langs, moviehash(w.path).as_deref());
        let resp = self
            .http
            .get(format!("{API}/subtitles"))
            .header("Api-Key", &self.key)
            .header("Accept", "application/json")
            .query(&query)
            .send()
            .await
            .context("opensubtitles search")?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            bail!("opensubtitles: {status} {}", short(&body));
        }
        Ok(hits(&body))
    }

    /// The text of one subtitle file. Each call counts against the
    /// account's downloads for the day.
    pub async fn download(&self, file_id: i64, lang: &str) -> Result<String> {
        let resp = self
            .http
            .post(format!("{API}/download"))
            .header("Api-Key", &self.key)
            .header("Accept", "application/json")
            .json(&serde_json::json!({ "file_id": file_id }))
            .send()
            .await
            .context("opensubtitles download")?;
        let status = resp.status();
        let body = resp.text().await?;
        let reply: DownloadReply = serde_json::from_str(&body).unwrap_or(DownloadReply {
            link: None,
            message: None,
        });
        let Some(link) = reply.link.filter(|_| status.is_success()) else {
            bail!(
                "opensubtitles: {status} {}",
                reply.message.unwrap_or_else(|| short(&body))
            );
        };
        let bytes = self
            .http
            .get(link)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        Ok(decode_text(&bytes, lang))
    }
}

fn search_query(w: &Wanted<'_>, langs: &str, hash: Option<&str>) -> Vec<(&'static str, String)> {
    // The API wants the languages lower case and sorted.
    let mut l: Vec<String> = langs
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    l.sort();
    l.dedup();
    let mut q = vec![("languages", l.join(","))];
    match (w.episode, w.tmdb_id) {
        (Some((s, e)), Some(id)) => {
            q.push(("parent_tmdb_id", id.to_string()));
            q.push(("season_number", s.to_string()));
            q.push(("episode_number", e.to_string()));
        }
        (Some((s, e)), None) => {
            q.push(("query", w.title.to_lowercase()));
            q.push(("season_number", s.to_string()));
            q.push(("episode_number", e.to_string()));
        }
        (None, Some(id)) => q.push(("tmdb_id", id.to_string())),
        (None, None) => {
            q.push(("query", w.title.to_lowercase()));
            if let Some(y) = w.year {
                q.push(("year", y.to_string()));
            }
        }
    }
    if let Some(h) = hash {
        q.push(("moviehash", h.to_string()));
    }
    q.push(("order_by", "download_count".into()));
    q
}

fn hits(body: &str) -> Vec<SubHit> {
    let Ok(reply) = serde_json::from_str::<SearchReply>(body) else {
        return vec![];
    };
    let mut out: Vec<SubHit> = reply
        .data
        .into_iter()
        .filter_map(|i| {
            let a = i.attributes;
            let file = a.files.first()?;
            Some(SubHit {
                file_id: file.file_id,
                language: a.language.unwrap_or_default(),
                release: a.release.unwrap_or_default(),
                downloads: a.download_count,
                exact: a.moviehash_match,
                hearing_impaired: a.hearing_impaired,
            })
        })
        .collect();
    out.sort_by(|a, b| b.exact.cmp(&a.exact).then(b.downloads.cmp(&a.downloads)));
    out
}

/// OpenSubtitles' hash: the size plus the 64-bit little-endian words of the
/// first and last 64 KiB. None for a file smaller than that.
pub fn moviehash(path: &Path) -> Option<String> {
    const CHUNK: u64 = 65536;
    let mut f = std::fs::File::open(path).ok()?;
    let size = f.metadata().ok()?.len();
    if size < CHUNK * 2 {
        return None;
    }
    let mut sum = size;
    let mut buf = vec![0u8; CHUNK as usize];
    for at in [0, size - CHUNK] {
        f.seek(SeekFrom::Start(at)).ok()?;
        f.read_exact(&mut buf).ok()?;
        for w in buf.chunks_exact(8) {
            sum = sum.wrapping_add(u64::from_le_bytes(w.try_into().ok()?));
        }
    }
    Some(format!("{sum:016x}"))
}

/// Subtitle files are often not UTF-8 but the old Windows code page of
/// their language (1250 for Croatian, say): read them in that.
fn decode_text(bytes: &[u8], lang: &str) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    let page = match lang.get(..2).unwrap_or("") {
        "hr" | "bs" | "sr" | "sl" | "cs" | "sk" | "pl" | "hu" | "ro" | "sq" => {
            encoding_rs::WINDOWS_1250
        }
        "ru" | "bg" | "uk" | "mk" | "be" => encoding_rs::WINDOWS_1251,
        "el" => encoding_rs::WINDOWS_1253,
        "tr" => encoding_rs::WINDOWS_1254,
        _ => encoding_rs::WINDOWS_1252,
    };
    page.decode(bytes).0.into_owned()
}

/// Writes `text` beside `video` as `Video.<lang>.srt`, or
/// `Video.<lang>.opensubtitles.srt` when that name is taken. Returns the path.
pub fn save_beside(video: &Path, lang: &str, text: &str) -> Result<PathBuf> {
    let lang: String = lang
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    let lang = if lang.is_empty() {
        "und".to_string()
    } else {
        lang.to_lowercase()
    };
    let stem = video
        .file_stem()
        .and_then(|s| s.to_str())
        .context("video has no name")?;
    let dir = video.parent().context("video has no folder")?;
    let mut path = dir.join(format!("{stem}.{lang}.srt"));
    if path.exists() {
        path = dir.join(format!("{stem}.{lang}.opensubtitles.srt"));
    }
    std::fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

fn short(body: &str) -> String {
    let b = body.trim();
    if b.chars().count() > 160 {
        b.chars().take(160).collect::<String>() + "…"
    } else {
        b.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_put_the_exact_match_first() {
        let body = r#"{"data":[
          {"id":"1","attributes":{"language":"en","release":"Stalker.1979.BluRay","download_count":900,
            "hearing_impaired":false,"moviehash_match":false,"files":[{"file_id":11,"file_name":"a.srt"}]}},
          {"id":"2","attributes":{"language":"hr","release":"Stalker 1979 1080p","download_count":40,
            "hearing_impaired":true,"moviehash_match":true,"files":[{"file_id":22,"file_name":"b.srt"}]}},
          {"id":"3","attributes":{"language":"en","download_count":5,"files":[]}}
        ]}"#;
        let h = hits(body);
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].file_id, 22);
        assert!(h[0].exact && h[0].hearing_impaired);
        assert_eq!(h[1].release, "Stalker.1979.BluRay");
        assert!(hits("not json").is_empty());
    }

    #[test]
    fn the_query_follows_what_is_known() {
        let p = Path::new("/nowhere.mkv");
        let film = Wanted {
            title: "Stalker",
            year: Some(1979),
            tmdb_id: None,
            episode: None,
            path: p,
        };
        let q = search_query(&film, "hr, EN", Some("00ff"));
        assert_eq!(q[0], ("languages", "en,hr".to_string()));
        assert!(q.contains(&("query", "stalker".to_string())));
        assert!(q.contains(&("year", "1979".to_string())));
        assert!(q.contains(&("moviehash", "00ff".to_string())));
        let ep = Wanted {
            title: "Twin Peaks",
            year: None,
            tmdb_id: Some(1920),
            episode: Some((1, 3)),
            path: p,
        };
        let q = search_query(&ep, "en", None);
        assert!(q.contains(&("parent_tmdb_id", "1920".to_string())));
        assert!(q.contains(&("episode_number", "3".to_string())));
        assert!(!q.iter().any(|(k, _)| *k == "query"));
    }

    #[test]
    fn the_hash_is_size_plus_both_ends() {
        let dir = std::env::temp_dir().join(format!("katalog-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("v.mkv");
        // All zero but one word at the start and one at the end.
        let mut data = vec![0u8; 200_000];
        data[0] = 1;
        let n = data.len();
        data[n - 8] = 2;
        std::fs::write(&f, &data).unwrap();
        assert_eq!(moviehash(&f).unwrap(), format!("{:016x}", 200_000u64 + 3));
        std::fs::write(&f, [0u8; 10]).unwrap();
        assert!(moviehash(&f).is_none());
        let saved = save_beside(&f, "HR", "1\n").unwrap();
        assert_eq!(saved, dir.join("v.hr.srt"));
        let again = save_beside(&f, "hr", "1\n").unwrap();
        assert_eq!(again, dir.join("v.hr.opensubtitles.srt"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn old_code_pages_are_read() {
        assert_eq!(decode_text(&[0xEF, 0xBB, 0xBF, b'a'], "en"), "a");
        assert_eq!(decode_text(&[b'z', 0xE8], "hr"), "zč");
        assert_eq!(decode_text(&[b'z', 0xE8], "fr"), "zè");
    }
}
