//! katalogd: the Katalog daemon.
//!
//! Owns the music library. Scans `~/Library/Music`, keeps SQLite in line with it,
//! finds art and lyrics, and serves everything over HTTP so any view can use it:
//! the desktop window, the CLI, xmobar, or a phone on the same network.
//!
//!   katalogd                       scan, watch and serve on 127.0.0.1:7300
//!   katalogd --bind 0.0.0.0:7300   also reachable from the phone
//!   katalogd --no-network          never call MusicBrainz or LRCLIB

mod api;
mod watch;

use anyhow::{Context, Result};
use clap::Parser;
use katalog_core::fetch::Fetcher;
use katalog_core::{db, Scanner};
use rusqlite_shim::Db;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info, warn};

#[derive(Parser, Debug, Clone)]
#[command(
    version,
    about = "The Katalog daemon: owns the music library, art and lyrics"
)]
pub struct Args {
    /// Root of the library. Music is read from <library>/Music.
    #[arg(long, env = "KATALOG_LIBRARY", default_value_os_t = default_library())]
    library: PathBuf,

    /// SQLite database file.
    #[arg(long, env = "KATALOG_DB", default_value_os_t = default_db())]
    db: PathBuf,

    /// Address for the HTTP API. Use 0.0.0.0:7300 to reach it from other devices.
    #[arg(long, env = "KATALOG_BIND", default_value = "127.0.0.1:7300")]
    bind: SocketAddr,

    /// Unix socket for local views. Empty disables it.
    #[arg(long, env = "KATALOG_SOCKET", default_value_os_t = default_socket())]
    socket: PathBuf,

    /// Do not look anything up online (art, lyrics).
    #[arg(long)]
    no_network: bool,

    /// Do not write fetched lyrics as .lrc files next to the music.
    #[arg(long)]
    no_lrc_files: bool,

    /// Scan once and exit, without serving.
    #[arg(long)]
    scan_only: bool,
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
fn default_library() -> PathBuf {
    home().join("Library")
}
fn default_db() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share"))
        .join("katalog/katalog.db")
}
fn default_socket() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("katalogd.sock")
}

/// Shared between the API, the scanner and the watcher.
pub struct State {
    pub db: Db,
    pub music_root: PathBuf,
    pub args: Args,
    pub scanning: std::sync::atomic::AtomicBool,
}

pub type Shared = Arc<State>;

/// The database connection behind a mutex. SQLite is fast enough for one process
/// to share a single connection; keeping it simple beats a pool here.
mod rusqlite_shim {
    use std::sync::{Mutex, MutexGuard};
    pub struct Db(Mutex<rusqlite::Connection>);
    impl Db {
        pub fn new(c: rusqlite::Connection) -> Self {
            Self(Mutex::new(c))
        }
        pub fn lock(&self) -> MutexGuard<'_, rusqlite::Connection> {
            self.0.lock().unwrap_or_else(|p| p.into_inner())
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();

    let args = Args::parse();
    let music_root = args.library.join("Music");
    if !music_root.is_dir() {
        anyhow::bail!(
            "{} does not exist. Point --library at the folder that holds Music/ (or set KATALOG_LIBRARY).",
            music_root.display()
        );
    }
    let conn = db::open(&args.db).context("open database")?;
    info!(
        "library {} · database {}",
        music_root.display(),
        args.db.display()
    );

    let state: Shared = Arc::new(State {
        db: Db::new(conn),
        music_root: music_root.clone(),
        args: args.clone(),
        scanning: std::sync::atomic::AtomicBool::new(false),
    });

    if args.scan_only {
        let report = run_scan(&state).await?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        if !args.no_network {
            fetch_missing(&state).await;
        }
        return Ok(());
    }

    // Initial scan in the background so the API answers right away.
    let s = state.clone();
    tokio::spawn(async move {
        if let Err(e) = run_scan(&s).await {
            error!("scan failed: {e:#}");
        }
        if !s.args.no_network {
            fetch_missing(&s).await;
        }
    });

    watch::start(state.clone());

    let app = api::router(state.clone());
    let tcp = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("bind {}", args.bind))?;
    info!("listening on http://{}", args.bind);

    let unix = if args.socket.as_os_str().is_empty() {
        None
    } else {
        let _ = std::fs::remove_file(&args.socket);
        match tokio::net::UnixListener::bind(&args.socket) {
            Ok(l) => {
                info!("listening on {}", args.socket.display());
                Some(l)
            }
            Err(e) => {
                error!("could not bind {}: {e}", args.socket.display());
                None
            }
        }
    };

    let tcp_server = axum::serve(tcp, app.clone()).with_graceful_shutdown(shutdown());
    match unix {
        Some(l) => {
            let unix_server = axum::serve(l, app).with_graceful_shutdown(shutdown());
            let (a, b) = tokio::join!(tcp_server, unix_server);
            a?;
            b?;
        }
        None => tcp_server.await?,
    }
    let _ = std::fs::remove_file(&args.socket);
    info!("bye");
    Ok(())
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}

/// Runs a full scan on a blocking thread. Only one scan runs at a time.
pub async fn run_scan(state: &Shared) -> Result<katalog_core::ScanReport> {
    scan_paths(state, None).await
}

/// Scans the given folders, or the whole music root when `dirs` is None.
pub async fn scan_paths(
    state: &Shared,
    dirs: Option<Vec<PathBuf>>,
) -> Result<katalog_core::ScanReport> {
    use std::sync::atomic::Ordering;
    if state.scanning.swap(true, Ordering::SeqCst) {
        anyhow::bail!("a scan is already running");
    }
    let s = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut conn = s.db.lock();
        let mut scanner = Scanner::new(&mut conn, s.music_root.clone());
        match dirs {
            Some(d) => scanner.scan_dirs(&d, true),
            None => scanner.scan_all(),
        }
    })
    .await?;
    state.scanning.store(false, Ordering::SeqCst);
    result
}

/// Looks up missing art and lyrics online. The database is locked only for the
/// short reads and writes, never while waiting on the network.
pub async fn fetch_missing(state: &Shared) {
    use katalog_core::fetch;
    let fetcher = match Fetcher::new() {
        Ok(f) => f,
        Err(e) => {
            error!("fetcher: {e:#}");
            return;
        }
    };
    let write_lrc = !state.args.no_lrc_files;
    let mut report = fetch::FetchReport::default();

    loop {
        let albums = match fetch::pending_albums(&state.db.lock(), 25) {
            Ok(a) => a,
            Err(e) => {
                error!("fetch: {e:#}");
                return;
            }
        };
        if albums.is_empty() {
            break;
        }
        for a in albums {
            match fetcher.cover_art(&a.artist, &a.title).await {
                Ok(Some((mime, data))) => {
                    if let Err(e) = fetch::store_art(&state.db.lock(), a.id, &mime, &data) {
                        error!("store art: {e:#}");
                    }
                    report.art_found += 1;
                    info!("art: {} · {}", a.artist, a.title);
                }
                Ok(None) => {
                    let _ = fetch::mark_art_missing(&state.db.lock(), a.id);
                    report.art_missing += 1;
                }
                Err(e) => {
                    warn!("art lookup failed for {} · {}: {e:#}", a.artist, a.title);
                    if is_offline(&e) {
                        return;
                    }
                }
            }
            // MusicBrainz asks for one request per second.
            tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        }
    }

    loop {
        let tracks = match fetch::pending_tracks(&state.db.lock(), 50) {
            Ok(t) => t,
            Err(e) => {
                error!("fetch: {e:#}");
                return;
            }
        };
        if tracks.is_empty() {
            break;
        }
        for t in tracks {
            match fetcher
                .lrclib(&t.artist, &t.title, &t.album, t.duration_ms)
                .await
            {
                Ok(Some(text)) => {
                    if let Err(e) = fetch::store_lyrics(&state.db.lock(), &t, &text, write_lrc) {
                        error!("store lyrics: {e:#}");
                    }
                    report.lyrics_found += 1;
                }
                Ok(None) => {
                    let _ = fetch::mark_lyrics_missing(&state.db.lock(), t.id);
                    report.lyrics_missing += 1;
                }
                Err(e) => {
                    warn!("lyrics lookup failed for {} · {}: {e:#}", t.artist, t.title);
                    if is_offline(&e) {
                        return;
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
    }

    if report.art_found + report.art_missing + report.lyrics_found + report.lyrics_missing > 0 {
        info!(
            "fetched {} covers and {} lyrics ({} albums and {} tracks had nothing online)",
            report.art_found, report.lyrics_found, report.art_missing, report.lyrics_missing
        );
    }
}

/// A connection error means we are offline: stop instead of failing every title.
fn is_offline(e: &anyhow::Error) -> bool {
    e.downcast_ref::<reqwest::Error>()
        .map(|r| r.is_connect() || r.is_timeout())
        .unwrap_or(false)
}
