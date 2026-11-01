//! HTTP API. JSON for the library, bytes for art, the files themselves for streaming.
//!
//!   GET  /api/status
//!   GET  /api/artists?q=
//!   GET  /api/albums?q=&artist=
//!   GET  /api/albums/{id}             album with its tracks
//!   GET  /api/tracks?q=&album=&limit=
//!   GET  /api/tracks/{id}
//!   GET  /api/tracks/{id}/lyrics
//!   POST /api/scan                    full rescan
//!   POST /api/fetch?kind=lyrics|art|all&retry=1   look up missing art and lyrics online
//!   GET  /art/{id}                    image bytes
//!   GET  /stream/{track_id}           the audio file, with Range support

use crate::Shared;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Request, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
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
        .route("/api/tracks/{id}/lyrics", get(lyrics))
        .route("/api/scan", post(scan))
        .route("/api/fetch", post(fetch))
        .route("/art/{id}", get(art))
        .route("/stream/{id}", get(stream))
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

async fn scan(State(s): State<Shared>) -> ApiResult<katalog_core::ScanReport> {
    let report = crate::run_scan(&s)
        .await
        .map_err(|e| ApiError(StatusCode::CONFLICT, format!("{e:#}")))?;
    if !s.args.no_network {
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
