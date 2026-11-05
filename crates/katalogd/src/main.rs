//! katalogd: the Katalog daemon.
//!
//! Owns the library. Scans `~/Library/Music`, `Movies` and `TV-Shows`, keeps
//! SQLite in line with them, finds art, lyrics and posters, and serves everything over HTTP so any view can use it:
//! the desktop window, the CLI, xmobar, or a phone on the same network.
//!
//!   katalogd                       scan, watch and serve on 127.0.0.1:7300
//!   katalogd --bind 0.0.0.0:7300   also reachable from the phone
//!   katalogd --no-network          never call MusicBrainz or LRCLIB
//!   katalogd --device hw:1,0       play through that card, bit-perfect

mod api;
mod onair;
mod player;
mod watch;

use anyhow::{Context, Result};
use clap::Parser;
use katalog_core::fetch::Fetcher;
use katalog_core::{db, Scanner};
use rusqlite_shim::Db;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error, info};

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

    /// Look up missing art and lyrics by itself after every scan. Off by default:
    /// run `katalog fetch` when you want it.
    #[arg(long)]
    auto_fetch: bool,

    /// Do not write fetched lyrics as .lrc files next to the music.
    #[arg(long)]
    no_lrc_files: bool,

    /// Scan once and exit, without serving.
    #[arg(long)]
    scan_only: bool,

    /// ALSA output device: auto, default, or hw:1,0 / hw:CARD=DAC (see `katalog devices`).
    /// hw: devices get the file's own rate and depth, nothing in between.
    #[arg(long, env = "KATALOG_DEVICE", default_value = "auto")]
    device: String,

    /// TMDB v3 API key or v4 read token, for film and TV details and posters.
    /// Without one, films and shows keep the names from their files.
    #[arg(long, env = "TMDB_KEY", hide_env_values = true)]
    tmdb_key: Option<String>,
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
    pub fetching: std::sync::atomic::AtomicBool,
    pub player: player::Player,
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

    let device = if args.device == "auto" {
        player::output::auto_device()
    } else {
        args.device.clone()
    };
    let player = player::Player::start(&args.db, device).context("start player")?;

    let state: Shared = Arc::new(State {
        db: Db::new(conn),
        player,
        music_root: music_root.clone(),
        args: args.clone(),
        scanning: std::sync::atomic::AtomicBool::new(false),
        fetching: std::sync::atomic::AtomicBool::new(false),
    });

    if args.scan_only {
        let report = run_scan(&state).await?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        if !args.no_network {
            match fetch_missing(&state, FetchKind::All, false).await {
                Ok(r) => println!("{}", serde_json::to_string_pretty(&r)?),
                Err(e) => error!("fetch: {e:#}"),
            }
        }
        return Ok(());
    }

    // Initial scan in the background so the API answers right away.
    let s = state.clone();
    tokio::spawn(async move {
        if let Err(e) = run_scan(&s).await {
            error!("scan failed: {e:#}");
        }
        if s.args.auto_fetch && !s.args.no_network {
            if let Err(e) = fetch_missing(&s, FetchKind::All, false).await {
                error!("fetch: {e:#}");
            }
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
        let library = &s.args.library;
        let video_roots = [library.join("Movies"), library.join("TV-Shows")];
        let mut conn = s.db.lock();
        let (music, video) = match &dirs {
            Some(d) => {
                let music: Vec<PathBuf> = d
                    .iter()
                    .filter(|p| p.starts_with(&s.music_root))
                    .cloned()
                    .collect();
                let video = d
                    .iter()
                    .any(|p| video_roots.iter().any(|r| p.starts_with(r)));
                (Some(music), video)
            }
            None => (None, true),
        };
        let mut report = match music {
            Some(m) if m.is_empty() => katalog_core::ScanReport::default(),
            Some(m) => Scanner::new(&mut conn, s.music_root.clone()).scan_dirs(&m, true)?,
            None => Scanner::new(&mut conn, s.music_root.clone()).scan_all()?,
        };
        if video {
            report.video = Some(katalog_core::video::scan(&mut conn, library)?);
        }
        Ok(report)
    })
    .await?;
    state.scanning.store(false, Ordering::SeqCst);
    result
}

/// What a fetch should look for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FetchKind {
    Art,
    Lyrics,
    /// Film and TV details and posters from TMDB.
    Video,
    All,
}

/// Looks up missing art and lyrics online. The database is locked only for the
/// short reads and writes, never while waiting on the network.
///
/// A lookup that errors (offline, a bad answer) is left alone and tried again on
/// the next run. Only a confirmed "nothing found" is remembered, and `retry`
/// forgets those too. One fetch runs at a time; a second call returns an error.
pub async fn fetch_missing(
    state: &Shared,
    kind: FetchKind,
    retry: bool,
) -> Result<katalog_core::fetch::FetchReport> {
    use std::sync::atomic::Ordering;
    if state.fetching.swap(true, Ordering::SeqCst) {
        anyhow::bail!("a fetch is already running");
    }
    let result = fetch_inner(state, kind, retry).await;
    state.fetching.store(false, Ordering::SeqCst);
    if let Ok(r) = &result {
        if r.art_found + r.art_missing + r.lyrics_found + r.lyrics_missing + r.failed > 0 {
            info!(
                "fetched {} covers and {} lyrics ({} albums and {} tracks had nothing online, {} lookups failed and will be retried)",
                r.art_found, r.lyrics_found, r.art_missing, r.lyrics_missing, r.failed
            );
        }
        if r.video_found + r.video_missing > 0 {
            info!(
                "TMDB: {} films and shows found, {} not known",
                r.video_found, r.video_missing
            );
        }
    }
    result
}

async fn fetch_inner(
    state: &Shared,
    kind: FetchKind,
    retry: bool,
) -> Result<katalog_core::fetch::FetchReport> {
    use katalog_core::fetch;
    let fetcher = Fetcher::new()?;
    let write_lrc = !state.args.no_lrc_files;
    let mut report = fetch::FetchReport::default();
    let want_art = matches!(kind, FetchKind::Art | FetchKind::All);
    let want_lyrics = matches!(kind, FetchKind::Lyrics | FetchKind::All);
    let tmdb_key = state.args.tmdb_key.clone().filter(|k| !k.trim().is_empty());
    if kind == FetchKind::Video && tmdb_key.is_none() {
        anyhow::bail!("no TMDB key: start katalogd with TMDB_KEY set (or --tmdb-key)");
    }
    let want_video = matches!(kind, FetchKind::Video | FetchKind::All) && tmdb_key.is_some();

    if retry {
        let conn = state.db.lock();
        if want_art {
            fetch::reset_art_tried(&conn)?;
        }
        if want_lyrics {
            fetch::reset_lyrics_tried(&conn)?;
        }
        if want_video {
            katalog_core::video::reset_meta_tried(&conn)?;
        }
    }

    if let (true, Some(key)) = (want_video, &tmdb_key) {
        fetch_video(state, key, &mut report).await?;
    }

    // Albums are fetched in batches; a failed one is skipped within this run
    // (so the batch does not spin on it) and comes back on the next run.
    let mut skip_albums: std::collections::HashSet<i64> = Default::default();
    if want_art {
        loop {
            let albums: Vec<_> = fetch::pending_albums(&state.db.lock(), 25 + skip_albums.len())?
                .into_iter()
                .filter(|a| !skip_albums.contains(&a.id))
                .collect();
            if albums.is_empty() {
                break;
            }
            for a in albums {
                match fetcher.cover_art(&a.artist, &a.title).await {
                    Ok(Some((mime, data))) => {
                        fetch::store_art(&state.db.lock(), a.id, &mime, &data)?;
                        report.art_found += 1;
                        debug!("art: {} · {}", a.artist, a.title);
                    }
                    Ok(None) => {
                        fetch::mark_art_missing(&state.db.lock(), a.id)?;
                        report.art_missing += 1;
                    }
                    Err(e) => {
                        debug!("art lookup failed for {} · {}: {e:#}", a.artist, a.title);
                        report.failed += 1;
                        skip_albums.insert(a.id);
                        if is_offline(&e) {
                            anyhow::bail!("offline: could not reach musicbrainz.org, try again when the network is up");
                        }
                    }
                }
                // MusicBrainz asks for one request per second.
                tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
            }
        }
    }

    let mut skip_tracks: std::collections::HashSet<i64> = Default::default();
    if want_lyrics {
        loop {
            let tracks: Vec<_> = fetch::pending_tracks(&state.db.lock(), 50 + skip_tracks.len())?
                .into_iter()
                .filter(|t| !skip_tracks.contains(&t.id))
                .collect();
            if tracks.is_empty() {
                break;
            }
            let done = report.lyrics_found + report.lyrics_missing + report.failed;
            if done > 0 {
                info!(
                    "lyrics: {} found, {} not online, {} failed so far",
                    report.lyrics_found, report.lyrics_missing, report.failed
                );
            }
            for t in tracks {
                match fetcher
                    .lrclib(&t.artist, &t.title, &t.album, t.duration_ms)
                    .await
                {
                    Ok(Some(text)) => {
                        fetch::store_lyrics(&state.db.lock(), &t, &text, write_lrc)?;
                        report.lyrics_found += 1;
                        debug!("lyrics: {} · {}", t.artist, t.title);
                    }
                    Ok(None) => {
                        fetch::mark_lyrics_missing(&state.db.lock(), t.id)?;
                        report.lyrics_missing += 1;
                        debug!("no lyrics online: {} · {}", t.artist, t.title);
                    }
                    Err(e) => {
                        debug!("lyrics lookup failed for {} · {}: {e:#}", t.artist, t.title);
                        report.failed += 1;
                        skip_tracks.insert(t.id);
                        if is_offline(&e) {
                            anyhow::bail!(
                                "offline: could not reach lrclib.net, try again when the network is up"
                            );
                        }
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }
        }
    }
    Ok(report)
}

/// Details and posters for films and shows TMDB has not been asked about yet.
async fn fetch_video(
    state: &Shared,
    key: &str,
    report: &mut katalog_core::fetch::FetchReport,
) -> Result<()> {
    use katalog_core::video::{self, Kind, Tmdb};
    let tmdb = Tmdb::new(key)?;
    for kind in [Kind::Movie, Kind::Show] {
        let mut skip: std::collections::HashSet<i64> = Default::default();
        loop {
            let batch: Vec<_> = {
                let conn = state.db.lock();
                match kind {
                    Kind::Movie => video::pending_movies(&conn, 25 + skip.len())?,
                    Kind::Show => video::pending_shows(&conn, 25 + skip.len())?,
                }
            }
            .into_iter()
            .filter(|p| !skip.contains(&p.id))
            .collect();
            if batch.is_empty() {
                break;
            }
            for p in batch {
                match tmdb.find(kind, &p.title, p.year).await {
                    Ok(Some(d)) => {
                        let poster = match &d.poster_path {
                            Some(path) => tmdb.poster(path).await.unwrap_or(None),
                            None => None,
                        };
                        video::store_details(&state.db.lock(), kind, p.id, &d, poster)?;
                        report.video_found += 1;
                        debug!("TMDB: {} ({:?})", p.title, p.year);
                    }
                    Ok(None) => {
                        video::mark_missing(&state.db.lock(), kind, p.id)?;
                        report.video_missing += 1;
                    }
                    Err(e) => {
                        if format!("{e:#}").contains("401") {
                            return Err(e);
                        }
                        debug!("TMDB lookup failed for {}: {e:#}", p.title);
                        report.failed += 1;
                        skip.insert(p.id);
                        if is_offline(&e) {
                            anyhow::bail!("offline: could not reach themoviedb.org, try again when the network is up");
                        }
                    }
                }
                // Well under TMDB's limit of about 40 requests a second.
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
        }
    }
    Ok(())
}

/// A connection error means we are offline: stop instead of failing every title.
fn is_offline(e: &anyhow::Error) -> bool {
    e.downcast_ref::<reqwest::Error>()
        .map(|r| r.is_connect() || r.is_timeout())
        .unwrap_or(false)
}
