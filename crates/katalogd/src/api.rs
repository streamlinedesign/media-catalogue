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
//!   GET  /art/{id}?size=256&crop=1    image bytes; with size, a JPEG no larger than that (crop: square)
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
//!
//!   GET  /api/playlists               name, track count, length
//!   POST /api/playlists               {"name": "chill"}
//!   GET  /api/playlists/{id}          with its tracks in order
//!   PUT  /api/playlists/{id}          {"name": "..."} rename
//!   DELETE /api/playlists/{id}
//!   POST /api/playlists/{id}/tracks   {"tracks":[ids]} or {"album":id} or {"artist":id}, appended
//!   DELETE /api/playlists/{id}/tracks/{position}
//!   GET  /api/tracks/{id}/playlists   ids of the playlists it is in
//!   "playlist": id works in /api/player/play and /api/player/queue too.
//!
//!   POST /api/radio                   {"playlist": id} or {} for the whole library
//!   DELETE /api/radio                 off

use crate::player::{Cmd, Radio, Source, Status as PlayerStatus};
use crate::Shared;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Request, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{delete, get, post};
use axum::Router;
use katalog_core::{db, video};
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
        .route("/api/albums/{id}/picks", get(album_picks))
        .route("/api/movies", get(movies))
        .route("/api/movies/{id}", get(movie))
        .route("/api/shows", get(shows))
        .route("/api/shows/{id}", get(show))
        .route("/api/movies/{id}/watch", get(watch_movie))
        .route("/api/episodes/{id}/watch", get(watch_episode))
        .route("/video/{kind}/{id}", get(video_file))
        .route("/video/{kind}/{id}/sub/{n}", get(video_sub))
        .route("/api/tracks", get(tracks))
        .route("/api/tracks/{id}", get(track))
        .route("/api/tracks/{id}/lyrics", get(lyrics).put(put_lyrics))
        .route("/api/tracks/{id}/lyrics/search", get(search_lyrics))
        .route("/api/tracks/{id}/playlists", get(track_playlists))
        .route("/api/playlists", get(playlists).post(create_playlist))
        .route(
            "/api/playlists/{id}",
            get(playlist).put(rename_playlist).delete(delete_playlist),
        )
        .route("/api/playlists/{id}/tracks", post(playlist_add))
        .route(
            "/api/playlists/{id}/tracks/{position}",
            delete(playlist_remove),
        )
        .route("/api/radio", post(radio_on).delete(radio_off))
        .route("/api/stations", get(stations))
        .route("/stations/{name}", get(station_stream))
        .route("/api/scan", post(scan))
        .route("/api/fetch", post(fetch))
        .route("/art/{id}", get(art))
        .route("/stream/{id}", get(stream))
        .route("/api/player", get(player_status))
        .route("/api/player/devices", get(player_devices))
        .route("/api/player/levels", get(player_levels))
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

async fn movies(
    State(s): State<Shared>,
    Query(p): Query<ListQuery>,
) -> ApiResult<Vec<video::Movie>> {
    Ok(Json(video::movies(&s.db.lock(), q(&p.q))?))
}

async fn movie(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<video::Movie> {
    Ok(Json(
        video::movie(&s.db.lock(), id)?.ok_or_else(|| not_found("film"))?,
    ))
}

async fn shows(State(s): State<Shared>, Query(p): Query<ListQuery>) -> ApiResult<Vec<video::Show>> {
    Ok(Json(video::shows(&s.db.lock(), q(&p.q))?))
}

#[derive(Serialize)]
struct ShowWithEpisodes {
    #[serde(flatten)]
    show: video::Show,
    episodes: Vec<video::Episode>,
}

async fn show(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<ShowWithEpisodes> {
    let conn = s.db.lock();
    let show = video::show(&conn, id)?.ok_or_else(|| not_found("show"))?;
    let episodes = video::episodes(&conn, id)?;
    Ok(Json(ShowWithEpisodes { show, episodes }))
}

/// How to watch a film or an episode: the file (as a local path and as a URL
/// on this server), its subtitle files, and mpv's `--sid` for English.
#[derive(Serialize)]
struct WatchAnswer {
    #[serde(flatten)]
    watch: video::Watch,
    url: String,
    sub_urls: Vec<String>,
    /// The mpv arguments for all of the above, local paths first choice.
    mpv: Vec<String>,
}

async fn watch_movie(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<WatchAnswer> {
    watch_answer(&s, "movie", None, Some(id)).await
}

async fn watch_episode(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<WatchAnswer> {
    watch_answer(&s, "episode", Some(id), None).await
}

async fn watch_answer(
    s: &Shared,
    kind: &'static str,
    episode: Option<i64>,
    movie: Option<i64>,
) -> ApiResult<WatchAnswer> {
    let (title, path, ms) =
        video::watch_target(&s.db.lock(), episode, movie)?.ok_or_else(|| not_found(kind))?;
    let w = tokio::task::spawn_blocking(move || video::watch(title, &path, ms))
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    let id = episode.or(movie).unwrap_or(0);
    let url = format!("/video/{kind}/{id}");
    let sub_urls = (0..w.sub_files.len())
        .map(|n| format!("{url}/sub/{n}"))
        .collect();
    let mpv = mpv_args(&w);
    Ok(Json(WatchAnswer {
        watch: w,
        url,
        sub_urls,
        mpv,
    }))
}

/// The arguments mpv gets, the file last. Subtitle files are handed over one
/// by one and mpv's own search is off, so the track numbers are the ones
/// `--sid` was worked out for.
fn mpv_args(w: &video::Watch) -> Vec<String> {
    let mut a = vec![
        format!("--force-media-title={}", w.title),
        "--sub-auto=no".into(),
        format!("--sid={}", w.sid),
    ];
    a.extend(w.sub_files.iter().map(|f| format!("--sub-file={f}")));
    a.push(w.path.clone());
    a
}

async fn video_path(s: &Shared, kind: &str, id: i64) -> Result<String, ApiError> {
    let conn = s.db.lock();
    let t = match kind {
        "movie" => video::watch_target(&conn, None, Some(id))?,
        "episode" => video::watch_target(&conn, Some(id), None)?,
        _ => None,
    };
    Ok(t.ok_or_else(|| not_found("video"))?.1)
}

async fn video_file(
    State(s): State<Shared>,
    Path((kind, id)): Path<(String, i64)>,
    req: Request<Body>,
) -> Result<Response, ApiError> {
    let path = video_path(&s, &kind, id).await?;
    let res = ServeFile::new(&path)
        .oneshot(req)
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(res.into_response())
}

async fn video_sub(
    State(s): State<Shared>,
    Path((kind, id, n)): Path<(String, i64, usize)>,
    req: Request<Body>,
) -> Result<Response, ApiError> {
    let path = video_path(&s, &kind, id).await?;
    let file = video::sidecar_subs(std::path::Path::new(&path))
        .into_iter()
        .nth(n)
        .ok_or_else(|| not_found("subtitle"))?
        .0;
    let res = ServeFile::new(&file)
        .oneshot(req)
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(res.into_response())
}

/// Four albums to go with this one, each with the rule that picked it.
async fn album_picks(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Vec<db::Pick>> {
    Ok(Json(db::picks(&s.db.lock(), id)?))
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

#[derive(Deserialize)]
struct ArtQuery {
    size: Option<u32>,
    /// With size: cut to a square first, so every cover in a grid is the same shape.
    /// `crop=1`, `crop=true` or `crop=yes`.
    #[serde(default, deserialize_with = "flag")]
    crop: bool,
}

/// A query flag: `1`, `true`, `yes` and `on` are true, anything else false.
fn flag<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let s = String::deserialize(d)?;
    Ok(matches!(
        s.to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    ))
}

async fn art(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Query(q): Query<ArtQuery>,
) -> Result<Response, ApiError> {
    let (mime, data) = match q.size.map(|n| n.clamp(32, 1024)) {
        None => db::art(&s.db.lock(), id)?.ok_or_else(|| not_found("art"))?,
        Some(size) => {
            // Cropped thumbnails are cached under a different key than plain ones.
            let key = if q.crop { size + 10_000 } else { size };
            let cached = db::art_thumb(&s.db.lock(), id, key)?;
            if let Some(t) = cached {
                ("image/jpeg".to_string(), t)
            } else {
                let (_, full) = db::art(&s.db.lock(), id)?.ok_or_else(|| not_found("art"))?;
                let crop = q.crop;
                let thumb = tokio::task::spawn_blocking(move || make_thumb(&full, size, crop))
                    .await
                    .map_err(|e| anyhow::anyhow!(e))??;
                db::insert_art_thumb(&s.db.lock(), id, key, &thumb)?;
                ("image/jpeg".to_string(), thumb)
            }
        }
    };
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

/// Every station, the library first, with what is on now and next.
async fn stations(State(s): State<Shared>) -> ApiResult<Vec<katalog_core::stations::OnAir>> {
    let now = katalog_core::stations::now_ms();
    Ok(Json(katalog_core::stations::on_air(&s.db.lock(), now)?))
}

/// A station as an endless WAV, from the moment of tuning in:
/// `/stations/chill.wav`, `/stations/everything.wav`, or a playlist id.
async fn station_stream(
    State(s): State<Shared>,
    Path(name): Path<String>,
) -> Result<Response, ApiError> {
    let found = crate::onair::resolve(&s.db.lock(), &name)?;
    let (playlist, name) = found.ok_or_else(|| not_found("station"))?;
    let rx = crate::onair::listen(s.clone(), playlist, name.clone());
    let body = Body::from_stream(futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|b| (Ok::<_, std::convert::Infallible>(b), rx))
    }));
    Response::builder()
        .header("content-type", "audio/wav")
        .header("cache-control", "no-store")
        .header(
            "icy-name",
            name.chars()
                .filter(|c| c.is_ascii() && !c.is_ascii_control())
                .collect::<String>(),
        )
        .body(body)
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// The spectrum as it sounds now: `{"bands": [0.0..1.0, ...]}`, low to high.
async fn player_levels(State(s): State<Shared>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "bands": s.player.levels() }))
}

async fn player_devices(State(_s): State<Shared>) -> Json<Vec<crate::player::output::Device>> {
    Json(crate::player::output::devices())
}

/// What to play: explicit track ids, or everything of an album, artist or
/// playlist, in order.
#[derive(Deserialize, Default)]
struct Selection {
    #[serde(default)]
    tracks: Vec<i64>,
    album: Option<i64>,
    artist: Option<i64>,
    playlist: Option<i64>,
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
    if let Some(playlist) = sel.playlist {
        if db::playlist(conn, playlist)?.is_none() {
            return Err(not_found("playlist"));
        }
        ids.extend(db::playlist_tracks(conn, playlist)?.iter().map(|t| t.id));
    }
    if ids.is_empty() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "nothing to play: give tracks, an album, an artist or a playlist".into(),
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
        source: if sel.playlist.is_some() {
            Source::Playlist
        } else {
            Source::Manual
        },
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

// ---- playlists ----------------------------------------------------------

#[derive(Serialize)]
struct PlaylistWithTracks {
    #[serde(flatten)]
    playlist: db::Playlist,
    tracks: Vec<db::Track>,
}

#[derive(Deserialize)]
struct NameBody {
    name: String,
}

async fn playlists(State(s): State<Shared>) -> ApiResult<Vec<db::Playlist>> {
    Ok(Json(db::playlists(&s.db.lock())?))
}

async fn create_playlist(
    State(s): State<Shared>,
    Json(body): Json<NameBody>,
) -> ApiResult<db::Playlist> {
    let made = db::create_playlist(&s.db.lock(), &body.name)
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, format!("{e:#}")))?;
    Ok(Json(made))
}

async fn playlist(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<PlaylistWithTracks> {
    let conn = s.db.lock();
    let playlist = db::playlist(&conn, id)?.ok_or_else(|| not_found("playlist"))?;
    let tracks = db::playlist_tracks(&conn, id)?;
    Ok(Json(PlaylistWithTracks { playlist, tracks }))
}

#[derive(Deserialize)]
struct PlaylistEdit {
    name: Option<String>,
    /// False takes the playlist off the radio dial.
    station: Option<bool>,
}

/// Renames a playlist and/or puts it on or off the dial.
async fn rename_playlist(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Json(body): Json<PlaylistEdit>,
) -> ApiResult<db::Playlist> {
    let conn = s.db.lock();
    db::playlist(&conn, id)?.ok_or_else(|| not_found("playlist"))?;
    if let Some(name) = &body.name {
        db::rename_playlist(&conn, id, name)
            .map_err(|e| ApiError(StatusCode::BAD_REQUEST, format!("{e:#}")))?;
    }
    if let Some(on) = body.station {
        db::set_playlist_station(&conn, id, on)?;
    }
    Ok(Json(
        db::playlist(&conn, id)?.ok_or_else(|| not_found("playlist"))?,
    ))
}

async fn delete_playlist(
    State(s): State<Shared>,
    Path(id): Path<i64>,
) -> ApiResult<serde_json::Value> {
    if !db::delete_playlist(&s.db.lock(), id)? {
        return Err(not_found("playlist"));
    }
    Ok(Json(serde_json::json!({ "deleted": id })))
}

async fn playlist_add(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    body: Option<Json<Selection>>,
) -> ApiResult<PlaylistWithTracks> {
    let sel = body.map(|b| b.0).unwrap_or_default();
    let conn = s.db.lock();
    db::playlist(&conn, id)?.ok_or_else(|| not_found("playlist"))?;
    let ids = resolve(&conn, &sel)?;
    db::add_to_playlist(&conn, id, &ids)?;
    let playlist = db::playlist(&conn, id)?.ok_or_else(|| not_found("playlist"))?;
    let tracks = db::playlist_tracks(&conn, id)?;
    Ok(Json(PlaylistWithTracks { playlist, tracks }))
}

async fn playlist_remove(
    State(s): State<Shared>,
    Path((id, position)): Path<(i64, usize)>,
) -> ApiResult<PlaylistWithTracks> {
    let conn = s.db.lock();
    db::playlist(&conn, id)?.ok_or_else(|| not_found("playlist"))?;
    if !db::remove_from_playlist(&conn, id, position)? {
        return Err(not_found("position"));
    }
    let playlist = db::playlist(&conn, id)?.ok_or_else(|| not_found("playlist"))?;
    let tracks = db::playlist_tracks(&conn, id)?;
    Ok(Json(PlaylistWithTracks { playlist, tracks }))
}

async fn track_playlists(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Vec<i64>> {
    Ok(Json(db::playlists_of_track(&s.db.lock(), id)?))
}

// ---- radio --------------------------------------------------------------

#[derive(Deserialize, Default)]
struct RadioBody {
    playlist: Option<i64>,
}

async fn radio_on(
    State(s): State<Shared>,
    body: Option<Json<RadioBody>>,
) -> ApiResult<PlayerStatus> {
    let body = body.map(|b| b.0).unwrap_or_default();
    let name = match body.playlist {
        Some(id) => {
            let p = db::playlist(&s.db.lock(), id)?.ok_or_else(|| not_found("playlist"))?;
            if p.track_count == 0 {
                return Err(ApiError(
                    StatusCode::BAD_REQUEST,
                    format!("the playlist {} is empty", p.name),
                ));
            }
            p.name
        }
        None => "Everything".to_string(),
    };
    s.player.send(Cmd::Radio(Radio {
        playlist: body.playlist,
        name,
    }))?;
    Ok(Json(settled(&s).await))
}

async fn radio_off(State(s): State<Shared>) -> ApiResult<PlayerStatus> {
    player_cmd(State(s), Cmd::RadioOff).await
}

/// The player thread picks commands up between audio blocks, so give it a
/// moment before answering with the new status.
async fn settled(s: &Shared) -> PlayerStatus {
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    s.player.status()
}

/// Scales an image down so its longest side is `size`, as a JPEG. With `crop`
/// the middle square is taken first.
fn make_thumb(data: &[u8], size: u32, crop: bool) -> anyhow::Result<Vec<u8>> {
    let mut img = image::load_from_memory(data)?;
    if crop {
        let (w, h) = (img.width(), img.height());
        let side = w.min(h);
        if side > 0 && w != h {
            img = img.crop_imm((w - side) / 2, (h - side) / 2, side, side);
        }
    }
    let small = img.thumbnail(size, size).into_rgb8();
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 86).encode_image(&small)?;
    Ok(out.into_inner())
}
