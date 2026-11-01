//! HTTP API. JSON for the library, bytes for art, the files themselves for streaming.
//!
//!   GET  /api/status
//!   GET  /api/artists?q=
//!   GET  /api/albums?q=&artist=
//!   GET  /api/albums/{id}             album with its tracks
//!   GET  /api/tracks?q=&album=&limit=
//!   GET  /api/tracks/{id}
//!   GET  /api/tracks/{id}/lyrics
//!   GET  /api/tracks/{id}/lyrics/search?q=&artist=   LRCLIB hits to pick from
//!   PUT  /api/tracks/{id}/lyrics      {"text": "...", "source": "lrclib"}  save by hand
//!   POST /api/scan                    full rescan
//!   POST /api/fetch?kind=lyrics|art|all&retry=1   look up missing art and lyrics online
//!   GET  /art/{id}                    image bytes
//!   GET  /stream/{track_id}           the audio file, with Range support
//!
//!   GET  /api/player                  state, track, position, queue ids, format
//!   GET  /api/player/queue            the queue as tracks, with the current index
//!   GET  /api/player/devices          ALSA outputs to pick from
//!   POST /api/player/play             {"tracks":[ids]} or {"album":id} or {"artist":id}, "start":n
//!   POST /api/player/queue            same body, appended
//!   DELETE /api/player/queue
//!   POST /api/player/{pause|resume|toggle|stop|next|prev}
//!   POST /api/player/seek?ms=         POST /api/player/volume?v=0.0..1.0
//!   POST /api/player/jump?index=      DELETE /api/player/queue/{index}

use crate::player::{Cmd, Status as PlayerStatus};
use crate::Shared;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Request, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{delete, get, post};
use axum::Router;
use katalog_core::db;
use serde::{Deserialize, Serialize};
use tower::ServiceExt;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeFile;

pub fn router(state: Shared) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);
    Router::new()
        .route("/", get(root))
        .route("/api/status", get(status))
        .route("/api/artists", get(artists))
        .route("/api/albums", get(albums))
        .route("/api/albums/{id}", get(album))
        .route("/api/tracks", get(tracks))
        .route("/api/tracks/{id}", get(track))
        .route("/api/tracks/{id}/lyrics", get(lyrics).put(put_lyrics))
        .route("/api/tracks/{id}/lyrics/search", get(search_lyrics))
        .route("/api/scan", post(scan))
        .route("/api/fetch", post(fetch))
        .route("/art/{id}", get(art))
        .route("/stream/{id}", get(stream))
        .route("/api/player", get(player_status))
        .route("/api/player/devices", get(player_devices))
        .route("/api/player/play", post(player_play))
        .route(
            "/api/player/queue",
            get(player_queue).post(player_add).delete(player_clear),
        )
        .route("/api/player/queue/{index}", delete(player_remove))
        .route("/api/player/pause", post(|s| player_cmd(s, Cmd::Pause)))
        .route("/api/player/resume", post(|s| player_cmd(s, Cmd::Resume)))
        .route("/api/player/toggle", post(|s| player_cmd(s, Cmd::Toggle)))
        .route("/api/player/stop", post(|s| player_cmd(s, Cmd::Stop)))
        .route("/api/player/next", post(|s| player_cmd(s, Cmd::Next)))
        .route("/api/player/prev", post(|s| player_cmd(s, Cmd::Prev)))
        .route("/api/player/seek", post(player_seek))
        .route("/api/player/volume", post(player_volume))
        .route("/api/player/jump", post(player_jump))
        .layer(cors)
        .with_state(state)
}

type ApiResult<T> = Result<Json<T>, ApiError>;

pub struct ApiError(StatusCode, String);

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}
fn not_found(what: &str) -> ApiError {
    ApiError(StatusCode::NOT_FOUND, format!("{what} not found"))
}

async fn root() -> &'static str {
    "katalogd. Try /api/status, /api/albums, /api/tracks?q=..."
}

#[derive(Serialize)]
struct Status {
    version: &'static str,
    library: String,
    scanning: bool,
    fetching: bool,
    #[serde(flatten)]
    stats: db::Stats,
}

async fn status(State(s): State<Shared>) -> ApiResult<Status> {
    let stats = db::stats(&s.db.lock())?;
    Ok(Json(Status {
        version: env!("CARGO_PKG_VERSION"),
        library: s.music_root.display().to_string(),
        scanning: s.scanning.load(std::sync::atomic::Ordering::Relaxed),
        fetching: s.fetching.load(std::sync::atomic::Ordering::Relaxed),
        stats,
    }))
}

#[derive(Deserialize, Default)]
struct ListQuery {
    q: Option<String>,
    artist: Option<i64>,
    album: Option<i64>,
    limit: Option<i64>,
}

fn q(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|x| !x.is_empty())
}

async fn artists(
    State(s): State<Shared>,
    Query(p): Query<ListQuery>,
) -> ApiResult<Vec<db::Artist>> {
    Ok(Json(db::artists(&s.db.lock(), q(&p.q))?))
}

async fn albums(State(s): State<Shared>, Query(p): Query<ListQuery>) -> ApiResult<Vec<db::Album>> {
    Ok(Json(db::albums(&s.db.lock(), q(&p.q), p.artist)?))
}

#[derive(Serialize)]
struct AlbumWithTracks {
    #[serde(flatten)]
    album: db::Album,
    tracks: Vec<db::Track>,
}

async fn album(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<AlbumWithTracks> {
    let conn = s.db.lock();
    let album = db::album(&conn, id)?.ok_or_else(|| not_found("album"))?;
    let tracks = db::tracks(&conn, None, Some(id), 10_000)?;
    Ok(Json(AlbumWithTracks { album, tracks }))
}

async fn tracks(State(s): State<Shared>, Query(p): Query<ListQuery>) -> ApiResult<Vec<db::Track>> {
    Ok(Json(db::tracks(
        &s.db.lock(),
        q(&p.q),
        p.album,
        p.limit.unwrap_or(500).clamp(1, 50_000),
    )?))
}

async fn track(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<db::Track> {
    Ok(Json(
        db::track(&s.db.lock(), id)?.ok_or_else(|| not_found("track"))?,
    ))
}

async fn lyrics(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<db::Lyrics> {
    Ok(Json(
        db::lyrics(&s.db.lock(), id)?.ok_or_else(|| not_found("lyrics"))?,
    ))
}

#[derive(Deserialize)]
struct LyricsSearch {
    q: String,
    artist: Option<String>,
}

/// Search hits for a track, so a view can offer "none of these fit, search again".
async fn search_lyrics(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Query(p): Query<LyricsSearch>,
) -> ApiResult<Vec<katalog_core::fetch::LyricsHit>> {
    if s.args.no_network {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "katalogd was started with --no-network".into(),
        ));
    }
    {
        let conn = s.db.lock();
        db::track(&conn, id)?.ok_or_else(|| not_found("track"))?;
    }
    let fetcher = katalog_core::fetch::Fetcher::new()?;
    let hits = fetcher
        .search_lyrics(&p.q, p.artist.as_deref())
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("lrclib: {e:#}")))?;
    Ok(Json(hits))
}

#[derive(Deserialize)]
struct NewLyrics {
    text: String,
    #[serde(default = "manual")]
    source: String,
}
fn manual() -> String {
    "manual".into()
}

/// Saves lyrics chosen by hand, replacing any earlier ones, and writes the .lrc.
async fn put_lyrics(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Json(body): Json<NewLyrics>,
) -> ApiResult<db::Lyrics> {
    if body.text.trim().is_empty() {
        return Err(ApiError(StatusCode::BAD_REQUEST, "empty lyrics".into()));
    }
    let conn = s.db.lock();
    let track = db::track(&conn, id)?.ok_or_else(|| not_found("track"))?;
    katalog_core::fetch::save_lyrics(
        &conn,
        id,
        &track.path,
        &body.text,
        &body.source,
        !s.args.no_lrc_files,
    )?;
    Ok(Json(
        db::lyrics(&conn, id)?.ok_or_else(|| not_found("lyrics"))?,
    ))
}

async fn scan(State(s): State<Shared>) -> ApiResult<katalog_core::ScanReport> {
    let report = crate::run_scan(&s)
        .await
        .map_err(|e| ApiError(StatusCode::CONFLICT, format!("{e:#}")))?;
    if s.args.auto_fetch && !s.args.no_network {
        let s2 = s.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::fetch_missing(&s2, crate::FetchKind::All, false).await {
                tracing::warn!("fetch after scan: {e:#}");
            }
        });
    }
    Ok(Json(report))
}

#[derive(Deserialize)]
struct FetchQuery {
    #[serde(default = "all")]
    kind: crate::FetchKind,
    #[serde(default)]
    retry: bool,
}
fn all() -> crate::FetchKind {
    crate::FetchKind::All
}

/// Runs a fetch now and answers when it is done. Failures are not remembered,
/// so running it again (when wifi is on) picks them up.
async fn fetch(
    State(s): State<Shared>,
    Query(p): Query<FetchQuery>,
) -> ApiResult<katalog_core::fetch::FetchReport> {
    if s.args.no_network {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "katalogd was started with --no-network".into(),
        ));
    }
    let report = crate::fetch_missing(&s, p.kind, p.retry)
        .await
        .map_err(|e| ApiError(StatusCode::CONFLICT, format!("{e:#}")))?;
    Ok(Json(report))
}

async fn art(State(s): State<Shared>, Path(id): Path<i64>) -> Result<Response, ApiError> {
    let (mime, data) = db::art(&s.db.lock(), id)?.ok_or_else(|| not_found("art"))?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&mime).unwrap_or(HeaderValue::from_static("image/jpeg")),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    Ok((headers, data).into_response())
}

/// Streams the audio file itself. ServeFile handles Range requests, so a phone
/// can seek, and the content type from the extension.
async fn stream(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    req: Request<Body>,
) -> Result<Response, ApiError> {
    let path = {
        let conn = s.db.lock();
        db::track(&conn, id)?
            .ok_or_else(|| not_found("track"))?
            .path
    };
    let res = ServeFile::new(&path)
        .oneshot(req)
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(res.into_response())
}

// ---- player -------------------------------------------------------------

async fn player_status(State(s): State<Shared>) -> Json<PlayerStatus> {
    Json(s.player.status())
}

async fn player_devices(State(_s): State<Shared>) -> Json<Vec<crate::player::output::Device>> {
    Json(crate::player::output::devices())
}

/// What to play: explicit track ids, or everything of an album or artist, in order.
#[derive(Deserialize, Default)]
struct Selection {
    #[serde(default)]
    tracks: Vec<i64>,
    album: Option<i64>,
    artist: Option<i64>,
    #[serde(default)]
    start: usize,
}

fn resolve(conn: &rusqlite::Connection, sel: &Selection) -> Result<Vec<i64>, ApiError> {
    let mut ids = sel.tracks.clone();
    if let Some(album) = sel.album {
        ids.extend(
            db::tracks(conn, None, Some(album), 10_000)?
                .iter()
                .map(|t| t.id),
        );
    }
    if let Some(artist) = sel.artist {
        for album in db::albums(conn, None, Some(artist))? {
            ids.extend(
                db::tracks(conn, None, Some(album.id), 10_000)?
                    .iter()
                    .map(|t| t.id),
            );
        }
    }
    if ids.is_empty() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "nothing to play: give tracks, an album or an artist".into(),
        ));
    }
    Ok(ids)
}

async fn player_play(
    State(s): State<Shared>,
    body: Option<Json<Selection>>,
) -> ApiResult<PlayerStatus> {
    let sel = body.map(|b| b.0).unwrap_or_default();
    let tracks = resolve(&s.db.lock(), &sel)?;
    s.player.send(Cmd::Play {
        tracks,
        start: sel.start,
    })?;
    Ok(Json(settled(&s).await))
}

async fn player_add(
    State(s): State<Shared>,
    body: Option<Json<Selection>>,
) -> ApiResult<PlayerStatus> {
    let sel = body.map(|b| b.0).unwrap_or_default();
    let tracks = resolve(&s.db.lock(), &sel)?;
    s.player.send(Cmd::Add { tracks })?;
    Ok(Json(settled(&s).await))
}

#[derive(Serialize)]
struct Queue {
    index: Option<usize>,
    tracks: Vec<db::Track>,
}

async fn player_queue(State(s): State<Shared>) -> ApiResult<Queue> {
    let st = s.player.status();
    let conn = s.db.lock();
    let mut tracks = Vec::with_capacity(st.queue.len());
    for id in &st.queue {
        if let Some(t) = db::track(&conn, *id)? {
            tracks.push(t);
        }
    }
    Ok(Json(Queue {
        index: st.index,
        tracks,
    }))
}

async fn player_clear(State(s): State<Shared>) -> ApiResult<PlayerStatus> {
    player_cmd(State(s), Cmd::Clear).await
}

async fn player_remove(
    State(s): State<Shared>,
    Path(index): Path<usize>,
) -> ApiResult<PlayerStatus> {
    player_cmd(State(s), Cmd::Remove(index)).await
}

async fn player_cmd(State(s): State<Shared>, cmd: Cmd) -> ApiResult<PlayerStatus> {
    s.player.send(cmd)?;
    Ok(Json(settled(&s).await))
}

#[derive(Deserialize)]
struct SeekQuery {
    ms: Option<u64>,
    secs: Option<f64>,
}

async fn player_seek(
    State(s): State<Shared>,
    Query(q): Query<SeekQuery>,
) -> ApiResult<PlayerStatus> {
    let ms =
        q.ms.or_else(|| q.secs.map(|x| (x.max(0.0) * 1000.0) as u64))
            .ok_or(ApiError(
                StatusCode::BAD_REQUEST,
                "give ms= or secs=".into(),
            ))?;
    player_cmd(State(s), Cmd::SeekMs(ms)).await
}

#[derive(Deserialize)]
struct VolumeQuery {
    v: f32,
}

async fn player_volume(
    State(s): State<Shared>,
    Query(q): Query<VolumeQuery>,
) -> ApiResult<PlayerStatus> {
    player_cmd(State(s), Cmd::Volume(q.v)).await
}

#[derive(Deserialize)]
struct JumpQuery {
    index: usize,
}

async fn player_jump(
    State(s): State<Shared>,
    Query(q): Query<JumpQuery>,
) -> ApiResult<PlayerStatus> {
    player_cmd(State(s), Cmd::Jump(q.index)).await
}

/// The player thread picks commands up between audio blocks, so give it a
/// moment before answering with the new status.
async fn settled(s: &Shared) -> PlayerStatus {
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    s.player.status()
}
