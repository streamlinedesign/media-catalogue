//! Books and comics over HTTP: the shelf, the pages, and your marks.
//!
//!   GET  /api/books?kind=book|comic&q=   books and comics; q also finds words in highlights and notes
//!   GET  /api/books/home                 what you are reading and the latest highlights
//!   GET  /api/books/{id}                 a book with its chapters, bookmarks and highlights
//!   GET  /api/books/{id}/chapters/{n}    an EPUB chapter as paragraphs, headings and pictures
//!   GET  /books/{id}/res?path=           a file inside an EPUB (its pictures)
//!   GET  /books/{id}/page/{n}?w=&dark=1  a page of a PDF or comic as an image, w pixels wide
//!   GET  /api/books/{id}/page/{n}/words  the words on a PDF page with their boxes
//!   PUT  /api/books/{id}/position        {"page":n,"block":n,"progress":0.4}
//!   POST /api/books/{id}/bookmarks       {"page":n,"block":n,"label":""}  toggles
//!   PUT  /api/bookmarks/{id}             {"label":"..."}      DELETE /api/bookmarks/{id}
//!   POST /api/books/{id}/highlights      {"page":n,"anchor":{..},"text":"..","color":"ochre","note":""}
//!   PUT  /api/highlights/{id}            {"color":"pink"} or {"note":"..."}   DELETE /api/highlights/{id}
//!   GET  /api/books/{id}/highlights.md   every highlight and note as Markdown

use crate::Shared;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post, put};
use axum::Router;
use katalog_core::books::{self, Block, Book, Bookmark, Chapter, Highlight, NewHighlight, Word};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/books", get(list))
        .route("/api/books/home", get(home))
        .route("/api/books/{id}", get(one))
        .route("/api/books/{id}/chapters/{n}", get(chapter))
        .route("/books/{id}/res", get(resource))
        .route("/books/{id}/page/{n}", get(page))
        .route("/api/books/{id}/page/{n}/words", get(words))
        .route("/api/books/{id}/position", put(position))
        .route("/api/books/{id}/bookmarks", post(bookmark))
        .route(
            "/api/bookmarks/{id}",
            put(rename_bookmark).delete(delete_bookmark),
        )
        .route("/api/books/{id}/highlights", post(add_highlight))
        .route(
            "/api/highlights/{id}",
            put(update_highlight).delete(delete_highlight),
        )
        .route("/api/books/{id}/highlights.md", get(markdown))
}

pub struct Error(StatusCode, String);
type R<T> = Result<Json<T>, Error>;

impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Self {
        Error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
    }
}
impl From<tokio::task::JoinError> for Error {
    fn from(e: tokio::task::JoinError) -> Self {
        Error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}
fn missing(what: &str) -> Error {
    Error(StatusCode::NOT_FOUND, format!("{what} not found"))
}

fn get_book(s: &Shared, id: i64) -> Result<Book, Error> {
    books::book(&s.db.lock(), id)?.ok_or_else(|| missing("book"))
}

#[derive(Deserialize, Default)]
struct ListQuery {
    kind: Option<String>,
    q: Option<String>,
}

async fn list(State(s): State<Shared>, Query(p): Query<ListQuery>) -> R<Vec<Book>> {
    let kind = p.kind.as_deref().filter(|k| *k == "book" || *k == "comic");
    let q = p.q.as_deref().map(str::trim).filter(|q| !q.is_empty());
    Ok(Json(books::books(&s.db.lock(), kind, q)?))
}

#[derive(Serialize)]
struct RecentHighlight {
    #[serde(flatten)]
    highlight: Highlight,
    book: String,
}

#[derive(Serialize)]
struct Home {
    reading: Vec<Book>,
    highlights: Vec<RecentHighlight>,
}

async fn home(State(s): State<Shared>) -> R<Home> {
    let conn = s.db.lock();
    let reading = books::reading_now(&conn, 6)?;
    let highlights = books::recent_highlights(&conn, 8)?
        .into_iter()
        .map(|(highlight, book)| RecentHighlight { highlight, book })
        .collect();
    Ok(Json(Home {
        reading,
        highlights,
    }))
}

#[derive(Serialize)]
struct Full {
    #[serde(flatten)]
    book: Book,
    chapters: Vec<Chapter>,
    bookmarks: Vec<Bookmark>,
    highlights: Vec<Highlight>,
}

async fn one(State(s): State<Shared>, Path(id): Path<i64>) -> R<Full> {
    let book = get_book(&s, id)?;
    let chapters = if book.format == "epub" {
        let p = PathBuf::from(&book.path);
        tokio::task::spawn_blocking(move || books::epub_info(&p))
            .await??
            .chapters
    } else {
        vec![]
    };
    let conn = s.db.lock();
    Ok(Json(Full {
        bookmarks: books::bookmarks(&conn, id)?,
        highlights: books::highlights(&conn, id)?,
        book,
        chapters,
    }))
}

#[derive(Serialize)]
struct ChapterOut {
    index: usize,
    title: String,
    blocks: Vec<Block>,
}

async fn chapter(State(s): State<Shared>, Path((id, n)): Path<(i64, usize)>) -> R<ChapterOut> {
    let book = get_book(&s, id)?;
    if book.format != "epub" {
        return Err(Error(
            StatusCode::BAD_REQUEST,
            "only an EPUB has chapters".into(),
        ));
    }
    let p = PathBuf::from(&book.path);
    let (ch, blocks) = tokio::task::spawn_blocking(move || books::epub_chapter(&p, n)).await??;
    Ok(Json(ChapterOut {
        index: n,
        title: ch.title,
        blocks,
    }))
}

#[derive(Deserialize)]
struct ResQuery {
    path: String,
}

fn bytes(mime: &str, data: Vec<u8>) -> Response {
    let mut r = data.into_response();
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime).unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400"),
    );
    r
}

async fn resource(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Query(q): Query<ResQuery>,
) -> Result<Response, Error> {
    let book = get_book(&s, id)?;
    let p = PathBuf::from(&book.path);
    let (mime, data) = tokio::task::spawn_blocking(move || books::epub_resource(&p, &q.path))
        .await?
        .map_err(|_| missing("file"))?;
    Ok(bytes(&mime, data))
}

#[derive(Deserialize)]
struct PageQuery {
    w: Option<u32>,
    #[serde(default)]
    dark: u8,
}

/// The dark page: Katalog's ink for the paper, its paper colour for the type,
/// both a little softer than the pure ones so a long read does not glare.
const DARK_PAGE: [u8; 3] = [0x15, 0x14, 0x0f];
const DARK_TYPE: [u8; 3] = [0xe2, 0xdd, 0xa4];

fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("katalog/pages")
}

/// One unpack at a time: the pages asked for while a comic unpacks wait
/// for it rather than each unpacking it again.
static UNPACK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// How many unpacked comics to keep around.
const UNPACKED_KEPT: usize = 4;

/// The folder a RAR comic is unpacked in, unpacking it the first time.
fn unpacked(book: &Book, stamp: u64) -> anyhow::Result<PathBuf> {
    let root = cache_dir().with_file_name("unpacked");
    let dir = root.join(format!("{}-{stamp}", book.id));
    if dir.is_dir() {
        return Ok(dir);
    }
    let _one = UNPACK.lock().unwrap_or_else(|e| e.into_inner());
    if dir.is_dir() {
        return Ok(dir);
    }
    std::fs::create_dir_all(&root)?;
    books::unpack_comic(std::path::Path::new(&book.path), &dir)?;
    // Keep the last few; the drawn pages stay in the page cache anyway.
    if let Ok(rd) = std::fs::read_dir(&root) {
        let mut all: Vec<(std::time::SystemTime, PathBuf)> = rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir() && e.path() != dir)
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        all.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, old) in all.into_iter().skip(UNPACKED_KEPT - 1) {
            let _ = std::fs::remove_dir_all(old);
        }
    }
    Ok(dir)
}

/// Draws one page: a PDF page through pdftoppm, a comic page from its
/// archive. Drawn pages are kept in ~/.cache/katalog/pages, so turning back
/// is instant.
fn render(book: &Book, n: i64, w: u32, dark: bool) -> anyhow::Result<(String, Vec<u8>)> {
    let pdf = book.format == "pdf";
    let ext = if pdf { "png" } else { "jpg" };
    let mime = if pdf { "image/png" } else { "image/jpeg" };
    // The cache key carries the size and mtime, so a changed file draws anew.
    let stamp = std::fs::metadata(&book.path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let file = cache_dir().join(format!(
        "{}-{stamp}-{n}-{w}{}.{ext}",
        book.id,
        if dark { "-dark" } else { "" }
    ));
    if let Ok(data) = std::fs::read(&file) {
        return Ok((mime.into(), data));
    }
    let path = std::path::Path::new(&book.path);
    let raw = if pdf {
        books::pdf_page(path, n, w)?
    } else if books::needs_unpacking(path, &book.format) {
        let dir = unpacked(book, stamp)?;
        let pages = books::unpacked_pages(&dir);
        let name = pages
            .get(n as usize)
            .ok_or_else(|| anyhow::anyhow!("no page {}", n + 1))?;
        std::fs::read(dir.join(name))?
    } else {
        let pages = books::comic_pages(path, &book.format)?;
        let name = pages
            .get(n as usize)
            .ok_or_else(|| anyhow::anyhow!("no page {}", n + 1))?;
        books::comic_page(path, &book.format, name)?
    };
    let mut img = image::load_from_memory(&raw)?;
    if !pdf && img.width() > w {
        // Small pages for the strip take the quick way down.
        img = if w <= 400 {
            img.thumbnail(w, u32::MAX)
        } else {
            img.resize(w, u32::MAX, image::imageops::FilterType::Triangle)
        };
    }
    let mut rgba = img.to_rgba8();
    if dark {
        books::darken(&mut rgba, DARK_PAGE, DARK_TYPE);
    }
    let mut out = std::io::Cursor::new(Vec::new());
    if pdf {
        rgba.write_to(&mut out, image::ImageFormat::Png)?;
    } else {
        image::DynamicImage::ImageRgba8(rgba)
            .to_rgb8()
            .write_to(&mut out, image::ImageFormat::Jpeg)?;
    }
    let data = out.into_inner();
    if std::fs::create_dir_all(cache_dir()).is_ok() {
        let _ = std::fs::write(&file, &data);
    }
    Ok((mime.into(), data))
}

async fn page(
    State(s): State<Shared>,
    Path((id, n)): Path<(i64, i64)>,
    Query(q): Query<PageQuery>,
) -> Result<Response, Error> {
    let book = get_book(&s, id)?;
    if book.format == "epub" {
        return Err(Error(
            StatusCode::BAD_REQUEST,
            "an EPUB has chapters, not pages".into(),
        ));
    }
    let w = q.w.unwrap_or(1400).clamp(200, 4000);
    let dark = q.dark == 1;
    let (mime, data) = tokio::task::spawn_blocking(move || render(&book, n, w, dark)).await??;
    Ok(bytes(&mime, data))
}

async fn words(State(s): State<Shared>, Path((id, n)): Path<(i64, i64)>) -> R<Vec<Word>> {
    let book = get_book(&s, id)?;
    if book.format != "pdf" {
        return Ok(Json(vec![]));
    }
    let p = PathBuf::from(&book.path);
    Ok(Json(
        tokio::task::spawn_blocking(move || books::pdf_words(&p, n)).await??,
    ))
}

#[derive(Deserialize)]
struct Position {
    page: i64,
    #[serde(default)]
    block: i64,
    #[serde(default)]
    progress: f64,
}

async fn position(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Json(p): Json<Position>,
) -> Result<StatusCode, Error> {
    get_book(&s, id)?;
    books::set_position(&s.db.lock(), id, p.page, p.block, p.progress)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct NewBookmark {
    page: i64,
    #[serde(default)]
    block: i64,
    #[serde(default)]
    label: String,
}

#[derive(Serialize)]
struct Toggled {
    id: Option<i64>,
}

async fn bookmark(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Json(b): Json<NewBookmark>,
) -> R<Toggled> {
    get_book(&s, id)?;
    Ok(Json(Toggled {
        id: books::toggle_bookmark(&s.db.lock(), id, b.page, b.block, b.label.trim())?,
    }))
}

#[derive(Deserialize)]
struct Label {
    label: String,
}

async fn rename_bookmark(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Json(l): Json<Label>,
) -> Result<StatusCode, Error> {
    if books::rename_bookmark(&s.db.lock(), id, l.label.trim())? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(missing("bookmark"))
    }
}

async fn delete_bookmark(
    State(s): State<Shared>,
    Path(id): Path<i64>,
) -> Result<StatusCode, Error> {
    if books::delete_bookmark(&s.db.lock(), id)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(missing("bookmark"))
    }
}

async fn add_highlight(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Json(h): Json<NewHighlight>,
) -> R<Highlight> {
    get_book(&s, id)?;
    if h.text.trim().is_empty() {
        return Err(Error(StatusCode::BAD_REQUEST, "nothing selected".into()));
    }
    Ok(Json(books::add_highlight(&s.db.lock(), id, &h)?))
}

#[derive(Deserialize)]
struct Change {
    color: Option<String>,
    note: Option<String>,
}

async fn update_highlight(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Json(c): Json<Change>,
) -> R<Highlight> {
    Ok(Json(
        books::update_highlight(&s.db.lock(), id, c.color.as_deref(), c.note.as_deref())?
            .ok_or_else(|| missing("highlight"))?,
    ))
}

async fn delete_highlight(
    State(s): State<Shared>,
    Path(id): Path<i64>,
) -> Result<StatusCode, Error> {
    if books::delete_highlight(&s.db.lock(), id)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(missing("highlight"))
    }
}

async fn markdown(State(s): State<Shared>, Path(id): Path<i64>) -> Result<Response, Error> {
    get_book(&s, id)?;
    let s2 = s.clone();
    let md = tokio::task::spawn_blocking(move || books::highlights_markdown(&s2.db.lock(), id))
        .await??;
    Ok(bytes("text/markdown; charset=utf-8", md.into_bytes()))
}
