//! katalog: the command line view of katalogd.
//!
//!   katalog status
//!   katalog artists [query]
//!   katalog albums [query]
//!   katalog tracks [query]
//!   katalog album <id>
//!   katalog lyrics <track id>
//!   katalog scan
//!   katalog fetch lyrics|art|all [--retry]
//!   katalog url <track id>      the stream URL, for mpv or a phone
//!
//!   katalog play <track ids...> | --album <id> | --artist <id> | --playlist <name>
//!   katalog add  <track ids...> | --album <id> | --artist <id> | --playlist <name>
//!   katalog now | toggle | pause | resume | next | prev | stop
//!   katalog seek 1:30 | 90 | +10 | -10
//!   katalog volume 80
//!   katalog queue | jump <n> | remove <n> | clear
//!   katalog devices
//!
//!   katalog playlists                       all of them
//!   katalog playlist <name>                 its tracks
//!   katalog playlist-new <name>
//!   katalog playlist-add <name> <track ids...> | --album <id> | --artist <id>
//!   katalog playlist-remove <name> <n>
//!   katalog playlist-rename <name> <new name>
//!   katalog playlist-delete <name>
//!   katalog stations                        every station, what is on it now, its stream
//!   katalog radio [playlist]                tune in to a playlist's station, or everything
//!   katalog radio --off
//!
//! Add --json to any command for the raw answer.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde_json::Value;

#[derive(Parser)]
#[command(version, about = "Command line view of the Katalog daemon")]
struct Cli {
    /// Where katalogd listens.
    #[arg(long, env = "KATALOG_URL", default_value = "http://127.0.0.1:7300")]
    url: String,
    /// Print the raw JSON instead of a table.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Library size, last scan, what is missing art or lyrics
    Status,
    /// Artists, optionally matching a query
    Artists {
        query: Option<String>,
    },
    /// Albums, optionally matching a query
    Albums {
        query: Option<String>,
    },
    /// Tracks matching a query
    Tracks {
        query: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: i64,
    },
    /// One album with its tracks
    Album {
        id: i64,
    },
    /// Lyrics of a track. When there are none it searches LRCLIB and lets you
    /// pick a hit or type a different search.
    Lyrics {
        id: i64,
        /// Search with this text instead of the track's title
        #[arg(long)]
        search: Option<String>,
        /// Save hit number N of the search without asking
        #[arg(long)]
        pick: Option<usize>,
    },
    /// Rescan the music folder now
    Scan,
    /// Look up missing lyrics or art online, now. Failures are retried next time.
    Fetch {
        /// lyrics, art or all
        #[arg(default_value = "all")]
        kind: String,
        /// Also look again at titles that had nothing online last time
        #[arg(long)]
        retry: bool,
    },
    /// The stream URL of a track
    Url {
        id: i64,
    },

    /// Play tracks now, replacing the queue
    Play {
        /// Track ids
        tracks: Vec<i64>,
        /// A whole album
        #[arg(long)]
        album: Option<i64>,
        /// Everything by an artist
        #[arg(long)]
        artist: Option<i64>,
        /// A playlist, by name or id
        #[arg(long)]
        playlist: Option<String>,
        /// Start at this position in the new queue
        #[arg(long, default_value_t = 0)]
        start: usize,
    },
    /// Add tracks to the end of the queue
    Add {
        tracks: Vec<i64>,
        #[arg(long)]
        album: Option<i64>,
        #[arg(long)]
        artist: Option<i64>,
        #[arg(long)]
        playlist: Option<String>,
    },
    /// What is playing, where, and the current lyric line
    Now,
    /// Pause or resume
    Toggle,
    Pause,
    Resume,
    Next,
    Prev,
    /// Stop and release the sound card
    Stop,
    /// Jump to a position: 1:30, 90, +10, -10
    Seek {
        to: String,
    },
    /// Volume 0 to 100. 100 leaves the samples untouched.
    Volume {
        percent: u8,
    },
    /// The queue, with the current track marked
    Queue,
    /// Play the n-th track of the queue (as listed by `queue`)
    Jump {
        n: usize,
    },
    /// Remove the n-th track of the queue
    Remove {
        n: usize,
    },
    /// Empty the queue and stop
    Clear,
    /// ALSA outputs katalogd can use with --device
    Devices,

    /// All playlists
    Playlists,
    /// The tracks of a playlist
    Playlist {
        /// Name or id
        name: String,
    },
    /// Make an empty playlist
    PlaylistNew {
        name: String,
    },
    /// Append tracks to a playlist (ones already in it are skipped)
    PlaylistAdd {
        name: String,
        tracks: Vec<i64>,
        #[arg(long)]
        album: Option<i64>,
        #[arg(long)]
        artist: Option<i64>,
    },
    /// Take entry n out of a playlist
    PlaylistRemove {
        name: String,
        n: usize,
    },
    PlaylistRename {
        name: String,
        new_name: String,
    },
    PlaylistDelete {
        name: String,
    },
    /// Put a playlist on the radio dial, or take it off
    PlaylistStation {
        name: String,
        #[arg(value_parser = ["on", "off"])]
        state: String,
    },
    /// Every station (each playlist, and the whole library) and what is on it now
    Stations,
    /// Tune in to a station: a playlist, or the whole library, joined live,
    /// part way into whatever song it is playing
    Radio {
        /// A playlist by name or id; none means everything
        playlist: Option<String>,
        /// Turn the radio off (what is playing goes on, the queue stops growing)
        #[arg(long)]
        off: bool,
    },
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let http = reqwest::Client::new();
    let base = cli.url.trim_end_matches('/');

    let get = |path: String| {
        let http = http.clone();
        let url = format!("{base}{path}");
        async move {
            let res = http
                .get(&url)
                .send()
                .await
                .with_context(|| format!("is katalogd running? could not reach {url}"))?;
            let status = res.status();
            let body: Value = res.json().await.unwrap_or(Value::Null);
            if !status.is_success() {
                anyhow::bail!(
                    "{status}: {}",
                    body.get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("request failed")
                );
            }
            Ok::<Value, anyhow::Error>(body)
        }
    };
    let enc = |s: &Option<String>| {
        s.as_deref()
            .map(|q| format!("?q={}", urlencoding(q)))
            .unwrap_or_default()
    };

    let value = match &cli.cmd {
        Cmd::Status => get("/api/status".into()).await?,
        Cmd::Artists { query } => get(format!("/api/artists{}", enc(query))).await?,
        Cmd::Albums { query } => get(format!("/api/albums{}", enc(query))).await?,
        Cmd::Tracks { query, limit } => {
            let sep = if query.is_some() { "&" } else { "?" };
            get(format!("/api/tracks{}{sep}limit={limit}", enc(query))).await?
        }
        Cmd::Album { id } => get(format!("/api/albums/{id}")).await?,
        Cmd::Lyrics { id, search, pick } => {
            let existing = if search.is_none() {
                match get(format!("/api/tracks/{id}/lyrics")).await {
                    Ok(v) => Some(v),
                    Err(e) if e.to_string().starts_with("404") => None,
                    Err(e) => return Err(e),
                }
            } else {
                None
            };
            match existing {
                Some(v) => v,
                None => {
                    let track = get(format!("/api/tracks/{id}")).await?;
                    return lyrics_search(&http, base, *id, &track, search.clone(), *pick).await;
                }
            }
        }
        Cmd::Scan => post(&http, format!("{base}/api/scan")).await?,
        Cmd::Fetch { kind, retry } => {
            if !["lyrics", "art", "all"].contains(&kind.as_str()) {
                anyhow::bail!("kind must be lyrics, art or all");
            }
            eprintln!("fetching {kind}, this answers when it is done ...");
            post(&http, format!("{base}/api/fetch?kind={kind}&retry={retry}")).await?
        }
        Cmd::Url { id } => {
            println!("{base}/stream/{id}");
            return Ok(());
        }
        Cmd::Play {
            tracks,
            album,
            artist,
            playlist,
            start,
        } => {
            let playlist = match playlist {
                Some(p) => Some(find_playlist(&http, base, p).await?["id"].clone()),
                None => None,
            };
            let body = serde_json::json!({"tracks": tracks, "album": album, "artist": artist, "playlist": playlist, "start": start});
            post_json(&http, format!("{base}/api/player/play"), body).await?
        }
        Cmd::Add {
            tracks,
            album,
            artist,
            playlist,
        } => {
            let playlist = match playlist {
                Some(p) => Some(find_playlist(&http, base, p).await?["id"].clone()),
                None => None,
            };
            let body = serde_json::json!({"tracks": tracks, "album": album, "artist": artist, "playlist": playlist});
            post_json(&http, format!("{base}/api/player/queue"), body).await?
        }
        Cmd::Playlists => get("/api/playlists".into()).await?,
        Cmd::Playlist { name } => {
            let p = find_playlist(&http, base, name).await?;
            get(format!("/api/playlists/{}", p["id"])).await?
        }
        Cmd::PlaylistNew { name } => {
            post_json(
                &http,
                format!("{base}/api/playlists"),
                serde_json::json!({ "name": name }),
            )
            .await?
        }
        Cmd::PlaylistAdd {
            name,
            tracks,
            album,
            artist,
        } => {
            let p = find_playlist(&http, base, name).await?;
            let body = serde_json::json!({"tracks": tracks, "album": album, "artist": artist});
            post_json(
                &http,
                format!("{base}/api/playlists/{}/tracks", p["id"]),
                body,
            )
            .await?
        }
        Cmd::PlaylistRemove { name, n } => {
            let p = find_playlist(&http, base, name).await?;
            delete(
                &http,
                format!(
                    "{base}/api/playlists/{}/tracks/{}",
                    p["id"],
                    n.saturating_sub(1)
                ),
            )
            .await?
        }
        Cmd::PlaylistRename { name, new_name } => {
            let p = find_playlist(&http, base, name).await?;
            put_json(
                &http,
                format!("{base}/api/playlists/{}", p["id"]),
                serde_json::json!({ "name": new_name }),
            )
            .await?
        }
        Cmd::PlaylistStation { name, state } => {
            let p = find_playlist(&http, base, name).await?;
            put_json(
                &http,
                format!("{base}/api/playlists/{}", p["id"]),
                serde_json::json!({ "station": state == "on" }),
            )
            .await?
        }
        Cmd::PlaylistDelete { name } => {
            let p = find_playlist(&http, base, name).await?;
            delete(&http, format!("{base}/api/playlists/{}", p["id"])).await?
        }
        Cmd::Stations => get("/api/stations".into()).await?,
        Cmd::Radio { playlist, off } => {
            if *off {
                delete(&http, format!("{base}/api/radio")).await?
            } else {
                let playlist = match playlist {
                    Some(p) => Some(find_playlist(&http, base, p).await?["id"].clone()),
                    None => None,
                };
                post_json(
                    &http,
                    format!("{base}/api/radio"),
                    serde_json::json!({ "playlist": playlist }),
                )
                .await?
            }
        }
        Cmd::Now => get("/api/player".into()).await?,
        Cmd::Toggle => post(&http, format!("{base}/api/player/toggle")).await?,
        Cmd::Pause => post(&http, format!("{base}/api/player/pause")).await?,
        Cmd::Resume => post(&http, format!("{base}/api/player/resume")).await?,
        Cmd::Next => post(&http, format!("{base}/api/player/next")).await?,
        Cmd::Prev => post(&http, format!("{base}/api/player/prev")).await?,
        Cmd::Stop => post(&http, format!("{base}/api/player/stop")).await?,
        Cmd::Seek { to } => {
            let ms = if let Some(rel) = to.strip_prefix('+').or_else(|| to.strip_prefix('-')) {
                let now = get("/api/player".into()).await?;
                let pos = now["position_ms"].as_i64().unwrap_or(0);
                let delta = parse_time(rel)? as i64;
                (if to.starts_with('+') {
                    pos + delta
                } else {
                    pos - delta
                })
                .max(0) as u64
            } else {
                parse_time(to)?
            };
            post(&http, format!("{base}/api/player/seek?ms={ms}")).await?
        }
        Cmd::Volume { percent } => {
            let v = (*percent).min(100) as f32 / 100.0;
            post(&http, format!("{base}/api/player/volume?v={v}")).await?
        }
        Cmd::Queue => get("/api/player/queue".into()).await?,
        Cmd::Jump { n } => {
            post(
                &http,
                format!("{base}/api/player/jump?index={}", n.saturating_sub(1)),
            )
            .await?
        }
        Cmd::Remove { n } => {
            delete(
                &http,
                format!("{base}/api/player/queue/{}", n.saturating_sub(1)),
            )
            .await?
        }
        Cmd::Clear => delete(&http, format!("{base}/api/player/queue")).await?,
        Cmd::Devices => get("/api/player/devices".into()).await?,
    };

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    match &cli.cmd {
        Cmd::Status => print_status(&value),
        Cmd::Artists { .. } => print_rows(&value, &["id", "name", "album_count", "track_count"]),
        Cmd::Albums { .. } => print_rows(&value, &["id", "artist", "title", "year", "track_count"]),
        Cmd::Tracks { .. } => print_rows(
            &value,
            &[
                "id",
                "artist",
                "album",
                "number",
                "title",
                "codec",
                "has_lyrics",
            ],
        ),
        Cmd::Album { .. } => {
            println!(
                "{} · {} ({})",
                s(&value["artist"]),
                s(&value["title"]),
                s(&value["year"])
            );
            print_rows(
                &value["tracks"],
                &["id", "number", "title", "duration_ms", "has_lyrics"],
            );
        }
        Cmd::Lyrics { .. } => {
            println!(
                "[{} · {}]\n",
                s(&value["source"]),
                if value["synced"].as_bool().unwrap_or(false) {
                    "synced"
                } else {
                    "plain"
                }
            );
            println!("{}", s(&value["text"]));
        }
        Cmd::Scan => println!(
            "{} files · {} added · {} updated · {} removed · {} unreadable · {:.1}s",
            s(&value["seen"]),
            s(&value["added"]),
            s(&value["updated"]),
            s(&value["removed"]),
            s(&value["unreadable"]),
            value["seconds"].as_f64().unwrap_or(0.0)
        ),
        Cmd::Fetch { .. } => println!(
            "{} covers found · {} lyrics found · {} albums and {} tracks have nothing online · {} lookups failed (run again later)",
            s(&value["art_found"]), s(&value["lyrics_found"]), s(&value["art_missing"]), s(&value["lyrics_missing"]), s(&value["failed"])
        ),
        Cmd::Url { .. } => {}
        Cmd::Now => {
            let lyric = current_lyric(&value, &get).await;
            print_now(&value, lyric.as_deref());
        }
        Cmd::Play { .. }
        | Cmd::Add { .. }
        | Cmd::Toggle
        | Cmd::Pause
        | Cmd::Resume
        | Cmd::Next
        | Cmd::Prev
        | Cmd::Stop
        | Cmd::Seek { .. }
        | Cmd::Volume { .. }
        | Cmd::Jump { .. }
        | Cmd::Remove { .. }
        | Cmd::Radio { .. }
        | Cmd::Clear => print_now(&value, None),
        Cmd::Stations => print_stations(&value, base),
        Cmd::Playlists => print_rows(
            &value,
            &["id", "name", "track_count", "duration_ms", "station"],
        ),
        Cmd::PlaylistStation { .. } => println!(
            "{}",
            if value["station"].as_bool().unwrap_or(true) {
                "on the dial"
            } else {
                "off the dial"
            }
        ),
        Cmd::Playlist { .. } | Cmd::PlaylistAdd { .. } | Cmd::PlaylistRemove { .. } => {
            print_playlist(&value)
        }
        Cmd::PlaylistNew { .. } | Cmd::PlaylistRename { .. } => {
            println!("{} (id {})", s(&value["name"]), s(&value["id"]))
        }
        Cmd::PlaylistDelete { .. } => println!("deleted"),
        Cmd::Queue => {
            let index = value["index"].as_u64();
            let Some(rows) = value["tracks"].as_array() else { return Ok(()) };
            for (i, t) in rows.iter().enumerate() {
                let mark = if Some(i as u64) == index { "▶" } else { " " };
                println!(
                    "{mark} {:>3}  {} · {} · {}",
                    i + 1,
                    s(&t["artist"]),
                    s(&t["title"]),
                    fmt_ms(t["duration_ms"].as_i64().unwrap_or(0))
                );
            }
            println!("{} tracks", rows.len());
        }
        Cmd::Devices => print_rows(&value, &["name", "description"]),
    }
    Ok(())
}

/// No lyrics stored: search LRCLIB, show the hits, let the user pick one or
/// retype the search (to drop the "(Remastered)" that gets in the way).
async fn lyrics_search(
    http: &reqwest::Client,
    base: &str,
    id: i64,
    track: &Value,
    first: Option<String>,
    pick: Option<usize>,
) -> Result<()> {
    let title = s(&track["title"]);
    let artist = s(&track["artist"]);
    let mut query = first.unwrap_or_else(|| title.clone());
    // The artist narrows the first search only; a typed query stands alone.
    let mut with_artist = true;
    println!("no lyrics stored for {artist} · {title}");
    loop {
        let mut url = format!(
            "{base}/api/tracks/{id}/lyrics/search?q={}",
            urlencoding(&query)
        );
        if with_artist && !artist.is_empty() {
            url.push_str(&format!("&artist={}", urlencoding(&artist)));
        }
        let res = http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("is katalogd running? could not reach {url}"))?;
        let hits = check(res).await?;
        let hits = hits.as_array().cloned().unwrap_or_default();
        println!(
            "\nsearch: {query}{}",
            if with_artist {
                format!(" · {artist}")
            } else {
                String::new()
            }
        );
        if hits.is_empty() {
            println!("nothing found");
        }
        for (i, h) in hits.iter().enumerate() {
            println!(
                "{:>3}  {} · {} · {} · {} · {}",
                i + 1,
                s(&h["artist"]),
                s(&h["title"]),
                s(&h["album"]),
                fmt_ms(h["duration_ms"].as_i64().unwrap_or(0)),
                if h["synced"].as_bool().unwrap_or(false) {
                    "synced"
                } else {
                    "plain"
                }
            );
        }
        let choice = match pick {
            Some(n) => n.to_string(),
            None => {
                eprint!("\nnumber to save, new search text, or Enter to stop: ");
                let mut line = String::new();
                std::io::stdin().read_line(&mut line)?;
                line.trim().to_string()
            }
        };
        if choice.is_empty() {
            return Ok(());
        }
        if let Ok(n) = choice.parse::<usize>() {
            let Some(h) = n.checked_sub(1).and_then(|i| hits.get(i)) else {
                if pick.is_some() {
                    anyhow::bail!("no hit {n}");
                }
                println!("no hit {n}");
                continue;
            };
            let body = serde_json::json!({ "text": h["text"], "source": h["source"] });
            let res = http
                .put(format!("{base}/api/tracks/{id}/lyrics"))
                .json(&body)
                .send()
                .await
                .context("is katalogd running?")?;
            let saved = check(res).await?;
            println!(
                "saved {} lyrics from {} ({} lines)",
                if saved["synced"].as_bool().unwrap_or(false) {
                    "synced"
                } else {
                    "plain"
                },
                s(&saved["source"]),
                s(&saved["text"]).lines().count()
            );
            return Ok(());
        }
        query = choice;
        with_artist = false;
    }
}

/// "1:30" → 90000, "90" → 90000, "2:05:00" → hours too.
fn parse_time(s: &str) -> Result<u64> {
    let mut total = 0f64;
    for part in s.split(':') {
        let v: f64 = part
            .trim()
            .parse()
            .with_context(|| format!("not a time: {s}"))?;
        total = total * 60.0 + v;
    }
    Ok((total.max(0.0) * 1000.0) as u64)
}

fn fmt_ms(ms: i64) -> String {
    let secs = ms.max(0) / 1000;
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

fn print_now(v: &Value, lyric: Option<&str>) {
    let state = s(&v["state"]);
    if let Some(e) = v["error"].as_str() {
        eprintln!("player: {e}");
    }
    if let Some(n) = v["notice"].as_str() {
        println!("{n}");
    }
    let radio = v["radio"]["name"]
        .as_str()
        .map(|n| format!(" · radio: {n}"))
        .unwrap_or_default();
    if v["track"].is_null() {
        println!(
            "stopped · {} in queue{radio}",
            v["queue"].as_array().map(|q| q.len()).unwrap_or(0)
        );
        return;
    }
    let t = &v["track"];
    println!("{} · {}", s(&t["artist"]), s(&t["title"]));
    println!(
        "{}{}",
        s(&t["album"]),
        t["year"]
            .as_i64()
            .map(|y| format!(" ({y})"))
            .unwrap_or_default()
    );
    let bar = {
        let pos = v["position_ms"].as_f64().unwrap_or(0.0);
        let dur = v["duration_ms"].as_f64().unwrap_or(0.0).max(1.0);
        let w = 30usize;
        let filled = ((pos / dur) * w as f64).round().clamp(0.0, w as f64) as usize;
        format!("{}{}", "━".repeat(filled), "─".repeat(w - filled))
    };
    println!(
        "{state} {} {bar} {}",
        fmt_ms(v["position_ms"].as_i64().unwrap_or(0)),
        fmt_ms(v["duration_ms"].as_i64().unwrap_or(0))
    );
    let mut line = format!("{} · {}", s(&v["format"]), s(&v["device"]));
    if v["bit_perfect"].as_bool().unwrap_or(false) {
        line.push_str(" · bit-perfect");
    }
    if let Some(vol) = v["volume"].as_f64() {
        if vol < 0.999 {
            line.push_str(&format!(" · volume {:.0}%", vol * 100.0));
        }
    }
    if let (Some(i), Some(q)) = (v["index"].as_u64(), v["queue"].as_array()) {
        line.push_str(&format!(" · {}/{} in queue", i + 1, q.len()));
    }
    line.push_str(&radio);
    println!("{line}");
    if let Some(l) = lyric {
        println!("\n{l}");
    }
}

/// The lyric line for the current position, when the lyrics are synced.
async fn current_lyric<F, Fut>(now: &Value, get: &F) -> Option<String>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<Value>>,
{
    let id = now["track"]["id"].as_i64()?;
    if !now["track"]["has_lyrics"].as_bool().unwrap_or(false) {
        return None;
    }
    let lyrics = get(format!("/api/tracks/{id}/lyrics")).await.ok()?;
    if !lyrics["synced"].as_bool().unwrap_or(false) {
        return None;
    }
    let pos = now["position_ms"].as_u64().unwrap_or(0);
    let text = lyrics["text"].as_str()?;
    let mut best: Option<(u64, &str)> = None;
    for line in text.lines() {
        let Some((stamp, rest)) = parse_lrc_line(line) else {
            continue;
        };
        if stamp <= pos && best.map(|(b, _)| stamp >= b).unwrap_or(true) {
            best = Some((stamp, rest));
        }
    }
    best.map(|(_, l)| l.trim().to_string())
        .filter(|l| !l.is_empty())
}

/// "[01:23.45]words" → (83450, "words").
fn parse_lrc_line(line: &str) -> Option<(u64, &str)> {
    let rest = line.strip_prefix('[')?;
    let end = rest.find(']')?;
    let (stamp, text) = (&rest[..end], &rest[end + 1..]);
    let (m, sec) = stamp.split_once(':')?;
    let m: u64 = m.parse().ok()?;
    let sec: f64 = sec.parse().ok()?;
    Some((m * 60_000 + (sec * 1000.0) as u64, text))
}

/// A playlist by id or name; the name need not match case.
async fn find_playlist(http: &reqwest::Client, base: &str, name: &str) -> Result<Value> {
    let res = http
        .get(format!("{base}/api/playlists"))
        .send()
        .await
        .with_context(|| format!("is katalogd running? could not reach {base}"))?;
    let all = check(res).await?;
    let all = all.as_array().cloned().unwrap_or_default();
    let by_id = name.parse::<i64>().ok();
    let found = all
        .iter()
        .find(|p| p["id"].as_i64() == by_id || s(&p["name"]).eq_ignore_ascii_case(name.trim()));
    match found {
        Some(p) => Ok(p.clone()),
        None => {
            let names: Vec<String> = all.iter().map(|p| s(&p["name"])).collect();
            anyhow::bail!(
                "no playlist called {name}{}",
                if names.is_empty() {
                    String::new()
                } else {
                    format!(" (there are: {})", names.join(", "))
                }
            )
        }
    }
}

fn print_playlist(v: &Value) {
    println!(
        "{} · {} tracks · {}",
        s(&v["name"]),
        s(&v["track_count"]),
        fmt_ms(v["duration_ms"].as_i64().unwrap_or(0))
    );
    let Some(rows) = v["tracks"].as_array() else {
        return;
    };
    for (i, t) in rows.iter().enumerate() {
        println!(
            "  {:>3}  {} · {} · {}  ({})",
            i + 1,
            s(&t["artist"]),
            s(&t["title"]),
            fmt_ms(t["duration_ms"].as_i64().unwrap_or(0)),
            s(&t["id"])
        );
    }
}

async fn put_json(http: &reqwest::Client, url: String, body: Value) -> Result<Value> {
    let res = http
        .put(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("is katalogd running? could not reach {url}"))?;
    check(res).await
}

async fn post_json(http: &reqwest::Client, url: String, body: Value) -> Result<Value> {
    let res = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("is katalogd running? could not reach {url}"))?;
    check(res).await
}

async fn delete(http: &reqwest::Client, url: String) -> Result<Value> {
    let res = http
        .delete(&url)
        .send()
        .await
        .with_context(|| format!("is katalogd running? could not reach {url}"))?;
    check(res).await
}

async fn check(res: reqwest::Response) -> Result<Value> {
    let status = res.status();
    let body: Value = res.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!(
            "{status}: {}",
            body.get("error")
                .and_then(Value::as_str)
                .unwrap_or("request failed")
        );
    }
    Ok(body)
}

async fn post(http: &reqwest::Client, url: String) -> Result<Value> {
    let res = http
        .post(&url)
        .send()
        .await
        .with_context(|| format!("is katalogd running? could not reach {url}"))?;
    check(res).await
}

fn print_stations(v: &Value, base: &str) {
    let Some(list) = v.as_array() else { return };
    for st in list {
        let slug = match st["playlist"].as_i64() {
            Some(id) => id.to_string(),
            None => "everything".into(),
        };
        let now = &st["now"];
        let on = if now.is_null() {
            "nothing to play".to_string()
        } else {
            format!(
                "{} · {}  {} / {}",
                s(&now["artist"]),
                s(&now["title"]),
                fmt_ms(now["position_ms"].as_i64().unwrap_or(0)),
                fmt_ms(now["duration_ms"].as_i64().unwrap_or(0)),
            )
        };
        println!("{:<20} {on}", s(&st["name"]));
        println!("{:<20} {base}/stations/{slug}.wav", "");
    }
}

fn s(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(x) => x.clone(),
        Value::Bool(b) => {
            if *b {
                "yes".into()
            } else {
                "".into()
            }
        }
        other => other.to_string(),
    }
}

fn print_status(v: &Value) {
    let hours = v["duration_ms"].as_f64().unwrap_or(0.0) / 3_600_000.0;
    let gb = v["bytes"].as_f64().unwrap_or(0.0) / 1e9;
    println!("katalogd {}  ·  {}", s(&v["version"]), s(&v["library"]));
    println!(
        "{} artists · {} albums · {} tracks · {hours:.1} h · {gb:.1} GB",
        s(&v["artists"]),
        s(&v["albums"]),
        s(&v["tracks"])
    );
    println!(
        "missing art: {} albums · missing lyrics: {} tracks",
        s(&v["albums_without_art"]),
        s(&v["tracks_without_lyrics"])
    );
    println!(
        "last scan: {}{}",
        s(&v["last_scan"]),
        if v["scanning"].as_bool().unwrap_or(false) {
            " (scanning now)"
        } else {
            ""
        }
    );
}

fn print_rows(v: &Value, cols: &[&str]) {
    let Some(rows) = v.as_array() else { return };
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            cols.iter()
                .map(|c| {
                    let x = &r[*c];
                    if *c == "duration_ms" {
                        let secs = x.as_i64().unwrap_or(0) / 1000;
                        format!("{}:{:02}", secs / 60, secs % 60)
                    } else {
                        s(x)
                    }
                })
                .collect()
        })
        .collect();
    let widths: Vec<usize> = (0..cols.len())
        .map(|i| {
            cells
                .iter()
                .map(|r| r[i].chars().count())
                .chain([cols[i].len()])
                .max()
                .unwrap_or(0)
                .min(48)
        })
        .collect();
    let line = |r: &[String]| {
        r.iter()
            .enumerate()
            .map(|(i, c)| format!("{:<w$}", trunc(c, widths[i]), w = widths[i]))
            .collect::<Vec<_>>()
            .join("  ")
    };
    println!(
        "{}",
        line(&cols.iter().map(|c| c.to_string()).collect::<Vec<_>>())
    );
    for r in &cells {
        println!("{}", line(r));
    }
    println!("{} rows", rows.len());
}

fn trunc(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else {
        s.chars().take(w.saturating_sub(1)).collect::<String>() + "…"
    }
}

fn urlencoding(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
