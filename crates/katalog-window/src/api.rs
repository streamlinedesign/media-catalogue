//! What katalogd answers, typed, and the calls that fetch it.

use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Album {
    pub id: i64,
    pub title: String,
    pub artist_id: i64,
    pub artist: String,
    pub year: Option<i64>,
    pub art_id: Option<i64>,
    pub track_count: i64,
    pub duration_ms: i64,
}

fn on_the_dial() -> bool {
    true
}

/// An album on the Home shelf and the rule that picked it.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Pick {
    #[serde(flatten)]
    pub album: Album,
    pub why: String,
}

/// A station and what is on it now.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct OnAir {
    /// The station's id. Daemons before 0.19 do not send one.
    #[serde(default)]
    pub id: Option<i64>,
    pub playlist: Option<i64>,
    /// For a genre station, its genre in lower case.
    #[serde(default)]
    pub genre: Option<String>,
    pub name: String,
    pub tracks: usize,
    pub now: Option<OnAirTrack>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct OnAirTrack {
    pub track_id: i64,
    pub title: String,
    pub artist: String,
    pub position_ms: i64,
    pub duration_ms: i64,
}

/// The spectrum, low to high, 0.0 to 1.0.
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct Levels {
    pub bands: Vec<f32>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Artist {
    pub id: i64,
    pub name: String,
    pub album_count: i64,
    pub track_count: i64,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Track {
    pub id: i64,
    pub title: String,
    pub album_id: i64,
    pub album: String,
    pub artist: String,
    pub disc: Option<i64>,
    pub number: Option<i64>,
    pub duration_ms: i64,
    pub codec: String,
    pub sample_rate: Option<i64>,
    pub bit_depth: Option<i64>,
    pub year: Option<i64>,
    pub art_id: Option<i64>,
    pub has_lyrics: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AlbumWithTracks {
    #[serde(flatten)]
    pub album: Album,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Copy)]
#[serde(rename_all = "lowercase")]
pub enum PlayState {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Player {
    pub state: PlayState,
    pub track: Option<Track>,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub queue: Vec<i64>,
    pub index: Option<usize>,
    pub volume: f32,
    pub device: String,
    pub format: Option<String>,
    pub bit_perfect: bool,
    pub error: Option<String>,
    #[serde(default)]
    pub radio: Option<Radio>,
    #[serde(default)]
    pub notice: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Radio {
    #[serde(default)]
    pub station: Option<i64>,
    pub playlist: Option<i64>,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub track_count: i64,
    pub duration_ms: i64,
    /// Has a station on the radio dial. Older daemons do not say, and had
    /// every playlist on it.
    #[serde(default = "on_the_dial")]
    pub station: bool,
    /// Made by katalogd from this genre; it refills it after each scan.
    #[serde(default)]
    pub genre: Option<String>,
}

impl Playlist {
    /// "12 tracks · 51 min", with "genre · " in front of a made one.
    pub fn meta(&self) -> String {
        format!(
            "{}{} {} · {}",
            if self.genre.is_some() {
                "genre · "
            } else {
                ""
            },
            self.track_count,
            if self.track_count == 1 {
                "track"
            } else {
                "tracks"
            },
            fmt_long(self.duration_ms)
        )
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PlaylistWithTracks {
    #[serde(flatten)]
    pub playlist: Playlist,
    pub tracks: Vec<Track>,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            state: PlayState::Stopped,
            track: None,
            position_ms: 0,
            duration_ms: 0,
            queue: Vec::new(),
            index: None,
            volume: 1.0,
            device: String::new(),
            format: None,
            bit_perfect: false,
            error: None,
            radio: None,
            notice: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Queue {
    pub index: Option<usize>,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Lyrics {
    pub track_id: i64,
    pub synced: bool,
    pub text: String,
    pub source: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct Status {
    pub version: String,
    pub library: String,
    pub scanning: bool,
    pub artists: i64,
    pub albums: i64,
    pub tracks: i64,
    pub duration_ms: i64,
    pub tracks_without_lyrics: i64,
    pub albums_without_art: i64,
    #[serde(default)]
    pub movies: i64,
    #[serde(default)]
    pub shows: i64,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Movie {
    pub id: i64,
    pub title: String,
    pub year: Option<i64>,
    pub duration_ms: i64,
    pub subs: Vec<String>,
    pub overview: Option<String>,
    pub released: Option<String>,
    pub genres: Vec<String>,
    pub poster_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Show {
    pub id: i64,
    pub title: String,
    pub year: Option<i64>,
    pub overview: Option<String>,
    pub first_aired: Option<String>,
    pub genres: Vec<String>,
    pub poster_id: Option<i64>,
    pub seasons: i64,
    pub episode_count: i64,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Episode {
    pub id: i64,
    pub season: i64,
    pub episode: i64,
    pub title: Option<String>,
    pub duration_ms: i64,
    pub subs: Vec<String>,
}

/// A TV channel and what is on it.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Channel {
    pub number: usize,
    pub name: String,
    pub items: usize,
    pub now: Option<Showing>,
    pub next: Option<Showing>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Showing {
    pub kind: String,
    pub id: i64,
    pub title: String,
    pub subtitle: Option<String>,
    pub year: Option<i64>,
    pub poster_id: Option<i64>,
    pub start_ms: i64,
    pub position_ms: i64,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Slot {
    pub start_ms: i64,
    pub duration_ms: i64,
}

/// How to join a channel now: `Watch` plus the slot.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ChannelWatch {
    #[serde(flatten)]
    pub watch: Watch,
    pub slot: Slot,
}

/// How katalogd says to watch something; see `/api/movies/{id}/watch`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Watch {
    pub title: String,
    pub path: String,
    pub url: String,
    pub sub_urls: Vec<String>,
    pub mpv: Vec<String>,
}

impl Watch {
    /// mpv's arguments, with the daemon's URLs in place of its paths when
    /// the files are not on this machine.
    pub fn args(&self, base: &str) -> Vec<String> {
        if std::path::Path::new(&self.path).exists() {
            return self.mpv.clone();
        }
        let mut subs = self.sub_urls.iter();
        self.mpv
            .iter()
            .map(|a| {
                if a.starts_with("--sub-file=") {
                    match subs.next() {
                        Some(u) => format!("--sub-file={base}{u}"),
                        None => a.clone(),
                    }
                } else if *a == self.path {
                    format!("{base}{}", self.url)
                } else {
                    a.clone()
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ShowWithEpisodes {
    #[serde(flatten)]
    pub show: Show,
    pub episodes: Vec<Episode>,
}

/// A film or episode left part way.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LeftOff {
    /// "movie" or "episode".
    pub kind: String,
    pub id: i64,
    pub title: String,
    pub subtitle: Option<String>,
    pub position_ms: i64,
    pub duration_ms: i64,
    pub poster_id: Option<i64>,
    pub open_id: i64,
}

/// A subtitle file on opensubtitles.com, from katalogd's search.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct SubHit {
    pub file_id: i64,
    pub language: String,
    #[serde(default)]
    pub release: String,
    #[serde(default)]
    pub downloads: i64,
    #[serde(default)]
    pub exact: bool,
    #[serde(default)]
    pub hearing_impaired: bool,
}

/// A search hit from LRCLIB, with its text ready to save.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LyricsHit {
    pub source: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: Option<i64>,
    pub synced: bool,
    pub text: String,
}

/// One line of synced lyrics.
#[derive(Debug, Clone, PartialEq)]
pub struct LyricLine {
    pub at_ms: Option<u64>,
    pub text: String,
}

impl Lyrics {
    pub fn lines(&self) -> Vec<LyricLine> {
        self.text
            .lines()
            .filter_map(|l| {
                let l = l.trim_end();
                if l.is_empty() {
                    return None;
                }
                match parse_lrc(l) {
                    Some((ms, text)) => {
                        // Metadata lines like [ar:], [ti:] have no time; drop them.
                        Some(LyricLine {
                            at_ms: Some(ms),
                            text: text.trim().to_string(),
                        })
                    }
                    None if l.starts_with('[') && l.contains(':') && l.contains(']') => None,
                    None => Some(LyricLine {
                        at_ms: None,
                        text: l.to_string(),
                    }),
                }
            })
            .collect()
    }
}

fn parse_lrc(line: &str) -> Option<(u64, &str)> {
    let rest = line.strip_prefix('[')?;
    let end = rest.find(']')?;
    let (stamp, text) = (&rest[..end], &rest[end + 1..]);
    let (m, sec) = stamp.split_once(':')?;
    let m: u64 = m.parse().ok()?;
    let sec: f64 = sec.parse().ok()?;
    Some((m * 60_000 + (sec * 1000.0) as u64, text))
}

#[derive(Clone)]
pub struct Client {
    pub base: Arc<String>,
    http: reqwest::Client,
}

pub type ApiResult<T> = Result<T, String>;

impl Client {
    pub fn new(base: String) -> Self {
        Self {
            base: Arc::new(base.trim_end_matches('/').to_string()),
            http: reqwest::Client::new(),
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    pub async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> ApiResult<T> {
        let url = self.url(path);
        let res = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|_| format!("katalogd is not running at {}", self.base))?;
        let status = res.status();
        if !status.is_success() {
            let body: serde_json::Value = res.json().await.unwrap_or_default();
            return Err(body["error"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| status.to_string()));
        }
        res.json::<T>()
            .await
            .map_err(|e| format!("bad answer: {e}"))
    }

    pub async fn bytes(&self, path: &str) -> ApiResult<Vec<u8>> {
        let url = self.url(path);
        let res = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;
        Ok(res.bytes().await.map_err(|e| e.to_string())?.to_vec())
    }

    /// POST with an optional JSON body; the player answers with its status.
    pub async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> ApiResult<T> {
        self.send(reqwest::Method::POST, path, body).await
    }

    pub async fn delete<T: serde::de::DeserializeOwned>(&self, path: &str) -> ApiResult<T> {
        self.send(reqwest::Method::DELETE, path, None).await
    }

    pub async fn put<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> ApiResult<T> {
        self.send(reqwest::Method::PUT, path, Some(body)).await
    }

    /// A request whose answer has no body (204).
    pub async fn send_ok(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> ApiResult<()> {
        let mut req = self.http.request(method, self.url(path));
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req
            .send()
            .await
            .map_err(|_| format!("katalogd is not running at {}", self.base))?;
        if res.status().is_success() {
            Ok(())
        } else {
            let status = res.status();
            let body: serde_json::Value = res.json().await.unwrap_or_default();
            Err(body["error"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| status.to_string()))
        }
    }

    async fn send<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> ApiResult<T> {
        let url = self.url(path);
        let mut req = self.http.request(method, &url);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req
            .send()
            .await
            .map_err(|_| format!("katalogd is not running at {}", self.base))?;
        let status = res.status();
        if !status.is_success() {
            let body: serde_json::Value = res.json().await.unwrap_or_default();
            return Err(body["error"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| status.to_string()));
        }
        res.json::<T>()
            .await
            .map_err(|e| format!("bad answer: {e}"))
    }
}

pub fn fmt_ms(ms: i64) -> String {
    let secs = ms.max(0) / 1000;
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

/// "2 h 14 min" for album and library lengths.
pub fn fmt_long(ms: i64) -> String {
    let mins = ms.max(0) / 60_000;
    if mins >= 60 {
        format!("{} h {} min", mins / 60, mins % 60)
    } else {
        format!("{mins} min")
    }
}
