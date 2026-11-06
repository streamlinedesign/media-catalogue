//! The phone page: music, films, shows and book downloads for a browser on
//! the same Wi-Fi, behind one password.
//!
//! It is a second listener (`--web 0.0.0.0:4180`) apart from the local API,
//! and serves only the read-only library routes (`api::library`) plus the
//! page itself. Nothing on it can change the library or touch the sound card.
//!
//! The password is `KATALOG_WEB_PASSWORD`. Signing in sets a cookie holding
//! a token made from the password and a random secret kept in the database,
//! so it survives a restart and changing the password signs everyone out.
//! The same token works as `?t=` for players that cannot send cookies (VLC).

use crate::api::{self, ApiError};
use crate::Shared;
use axum::body::Body;
use axum::extract::{Form, Path, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use katalog_core::{db, video};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;

const PAGE: &str = include_str!("web/index.html");
const LOGIN: &str = include_str!("web/login.html");
const COOKIE: &str = "katalog";

pub struct Web {
    password: String,
    token: String,
    /// Held across the pause after a wrong password, so guesses go one at a time.
    attempts: tokio::sync::Mutex<()>,
}

impl Web {
    pub fn new(state: &Shared, password: String) -> anyhow::Result<Arc<Web>> {
        let secret = {
            let conn = state.db.lock();
            let had: Option<String> = conn
                .query_row("SELECT value FROM meta WHERE key = 'web_secret'", [], |r| {
                    r.get(0)
                })
                .ok();
            match had {
                Some(s) if s.len() >= 32 => s,
                _ => {
                    let s = random_hex(32)?;
                    db::set_meta(&conn, "web_secret", &s)?;
                    s
                }
            }
        };
        let token = token_for(&secret, &password);
        Ok(Arc::new(Web {
            password,
            token,
            attempts: tokio::sync::Mutex::new(()),
        }))
    }

    fn allows(&self, headers: &HeaderMap, query: Option<&str>) -> bool {
        let from_cookie = headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(';'))
            .filter_map(|kv| kv.trim().split_once('='))
            .any(|(k, v)| k == COOKIE && same(v, &self.token));
        let from_query = query
            .unwrap_or("")
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .any(|(k, v)| k == "t" && same(v, &self.token));
        from_cookie || from_query
    }
}

fn random_hex(n: usize) -> anyhow::Result<String> {
    use std::io::Read;
    let mut buf = vec![0u8; n];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(hex(&buf))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn token_for(secret: &str, password: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"katalog-web\0");
    h.update(secret.as_bytes());
    h.update(b"\0");
    h.update(password.as_bytes());
    hex(&h.finalize())
}

/// Compares without stopping at the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

pub fn router(state: Shared, web: Arc<Web>) -> Router {
    let open = Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .with_state(web.clone());
    let page_web = web.clone();
    let guarded = api::library()
        .route(
            "/",
            get(move || {
                let t = page_web.token.clone();
                async move { page(&t) }
            }),
        )
        .route("/web/subs/{kind}/{id}", get(subs_list))
        .route("/web/sub/{kind}/{id}/{n}", get(sub_vtt))
        .with_state(state)
        .layer(middleware::from_fn_with_state(web, guard));
    guarded
        .merge(open)
        .layer(axum::middleware::map_response(no_store))
}

async fn guard(State(web): State<Arc<Web>>, req: Request, next: Next) -> Response {
    if web.allows(req.headers(), req.uri().query()) {
        return next.run(req).await;
    }
    if req.uri().path() == "/" {
        return login_page("").into_response();
    }
    (StatusCode::UNAUTHORIZED, "sign in first").into_response()
}

/// The page and the sign-in answers are never cached; covers and files are
/// left to the browser.
async fn no_store(mut res: Response) -> Response {
    let html = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/html"));
    if html {
        res.headers_mut()
            .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    }
    res
}

fn page(token: &str) -> Html<String> {
    Html(PAGE.replace("__TOKEN__", token))
}

fn login_page(note: &str) -> Html<String> {
    Html(LOGIN.replace("__NOTE__", note))
}

#[derive(Deserialize)]
struct LoginForm {
    password: String,
}

async fn login(State(web): State<Arc<Web>>, Form(f): Form<LoginForm>) -> Response {
    let _one_at_a_time = web.attempts.lock().await;
    if !same(&f.password, &web.password) {
        tokio::time::sleep(Duration::from_secs(1)).await;
        return (
            StatusCode::UNAUTHORIZED,
            login_page("That is not the password."),
        )
            .into_response();
    }
    let cookie = format!(
        "{COOKIE}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=31536000",
        web.token
    );
    ([(header::SET_COOKIE, cookie)], Redirect::to("/")).into_response()
}

async fn logout() -> Response {
    let cookie = format!("{COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    ([(header::SET_COOKIE, cookie)], Redirect::to("/")).into_response()
}

#[derive(serde::Serialize)]
struct SubFile {
    n: usize,
    lang: String,
}

/// The subtitle files beside a video that a browser can show once turned
/// into WebVTT: SubRip and WebVTT, not ASS.
async fn subs_list(
    State(s): State<Shared>,
    Path((kind, id)): Path<(String, i64)>,
) -> Result<axum::Json<Vec<SubFile>>, ApiError> {
    let path = api::video_path(&s, &kind, id).await?;
    let list = video::sidecar_subs(std::path::Path::new(&path))
        .into_iter()
        .enumerate()
        .filter(|(_, (p, _))| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("srt") || e.eq_ignore_ascii_case("vtt"))
        })
        .map(|(n, (_, lang))| SubFile { n, lang })
        .collect();
    Ok(axum::Json(list))
}

/// A subtitle file beside the video, as WebVTT, which is all a browser's
/// `<track>` reads. SubRip only differs in the header and the comma.
async fn sub_vtt(
    State(s): State<Shared>,
    Path((kind, id, n)): Path<(String, i64, usize)>,
) -> Result<Response, ApiError> {
    let path = api::video_path(&s, &kind, id).await?;
    let (file, lang) = video::sidecar_subs(std::path::Path::new(&path))
        .into_iter()
        .nth(n)
        .ok_or_else(|| api::not_found("subtitle"))?;
    let bytes = tokio::fs::read(&file)
        .await
        .map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?;
    let text = katalog_core::subs::decode_text(&bytes, &lang);
    let is_vtt = file
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("vtt"));
    let out = if is_vtt { text } else { srt_to_vtt(&text) };
    Ok((
        [(header::CONTENT_TYPE, "text/vtt; charset=utf-8")],
        Body::from(out),
    )
        .into_response())
}

fn srt_to_vtt(srt: &str) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for line in srt.trim_start_matches('\u{feff}').lines() {
        let line = line.trim_end_matches('\r');
        if line.contains("-->") {
            out.push_str(&line.replace(',', "."));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// The address a phone on the same network would use, when bound to all
/// interfaces. No packet is sent: connecting a UDP socket only picks a route.
pub fn lan_url(bind: std::net::SocketAddr) -> String {
    if !bind.ip().is_unspecified() {
        return format!("http://{bind}");
    }
    let ip = std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("192.0.2.1:9")?;
            s.local_addr()
        })
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "<this computer>".into());
    format!("http://{ip}:{}", bind.port())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subrip_becomes_webvtt() {
        let srt = "1\r\n00:00:01,000 --> 00:00:02,500\r\nHello, there\r\n\r\n";
        let vtt = srt_to_vtt(srt);
        assert!(vtt.starts_with("WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.500\n"));
        assert!(vtt.contains("Hello, there"), "commas in the text stay");
    }

    #[test]
    fn tokens_follow_the_password() {
        let a = token_for("s", "one");
        assert_eq!(a, token_for("s", "one"));
        assert_ne!(a, token_for("s", "two"));
        assert_ne!(a, token_for("t", "one"));
        assert!(same(&a, &a.clone()));
        assert!(!same(&a, &a[1..]));
    }

    #[test]
    fn cookie_or_query_lets_in() {
        let web = Web {
            password: "pw".into(),
            token: "abc".into(),
            attempts: tokio::sync::Mutex::new(()),
        };
        let mut h = HeaderMap::new();
        assert!(!web.allows(&h, None));
        assert!(web.allows(&h, Some("x=1&t=abc")));
        assert!(!web.allows(&h, Some("t=abd")));
        h.insert(header::COOKIE, "other=1; katalog=abc".parse().unwrap());
        assert!(web.allows(&h, None));
    }
}
