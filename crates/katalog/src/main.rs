//! katalog: the command line view of katalogd.
//!
//!   katalog status
//!   katalog artists [query]
//!   katalog albums [query]
//!   katalog tracks [query]
//!   katalog album <id>
//!   katalog lyrics <track id>
//!   katalog scan
//!   katalog url <track id>      the stream URL, for mpv or a phone
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
    Artists { query: Option<String> },
    /// Albums, optionally matching a query
    Albums { query: Option<String> },
    /// Tracks matching a query
    Tracks {
        query: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: i64,
    },
    /// One album with its tracks
    Album { id: i64 },
    /// Lyrics of a track
    Lyrics { id: i64 },
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
    Url { id: i64 },
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
        Cmd::Lyrics { id } => get(format!("/api/tracks/{id}/lyrics")).await?,
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
    }
    Ok(())
}

async fn post(http: &reqwest::Client, url: String) -> Result<Value> {
    let res = http
        .post(&url)
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
    Ok(body)
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
