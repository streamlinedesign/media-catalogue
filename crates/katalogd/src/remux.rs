//! MKV for Safari: the phone page hands a film to Safari as HLS, repackaged
//! by ffmpeg on the fly. Video is copied as it is (H.264 or HEVC, tagged
//! `hvc1` so Apple plays it); audio is copied when Safari knows it (AAC,
//! AC-3, E-AC-3) and only otherwise turned into stereo AAC, which costs
//! little. Nothing is scaled or re-encoded, so a 1.4 GB film takes about as
//! long as reading it from disk.
//!
//! Each film gets a folder under `~/.cache/katalog/hls/` with `index.m3u8`,
//! `init.mp4` and the segments; ffmpeg writes the playlist as it goes and
//! Safari starts as soon as the first segment is there. The last three
//! films are kept, so a film watched again starts at once.
//!
//! Text subtitles inside the file (SubRip, ASS, mov_text, WebVTT) come out
//! as WebVTT the same way, cached beside them.

use crate::api::{self, ApiError};
use crate::Shared;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Films kept in the cache besides the ones being made.
const KEEP: usize = 3;
/// How long the first request waits for ffmpeg's first segment.
const FIRST_SEGMENT: Duration = Duration::from_secs(40);

static JOBS: LazyLock<Mutex<HashMap<String, Child>>> = LazyLock::new(Default::default);

pub fn cache_root() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".cache")
        })
        .join("katalog/hls")
}

fn fail(code: StatusCode, msg: impl Into<String>) -> ApiError {
    ApiError(code, msg.into())
}

fn key(kind: &str, id: i64) -> Result<String, ApiError> {
    match kind {
        "movie" | "episode" => Ok(format!("{kind}-{id}")),
        _ => Err(api::not_found("video")),
    }
}

// ---- probing ----------------------------------------------------------------

#[derive(Deserialize, Default)]
struct ProbeOut {
    #[serde(default)]
    streams: Vec<ProbeStream>,
}

#[derive(Deserialize, Default, Clone)]
struct ProbeStream {
    #[serde(default)]
    codec_type: String,
    #[serde(default)]
    codec_name: String,
    #[serde(default)]
    tags: HashMap<String, String>,
    #[serde(default)]
    disposition: HashMap<String, i64>,
}

fn probe(path: &FsPath) -> Result<Vec<ProbeStream>, ApiError> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_streams", "-of", "json"])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| missing_ffmpeg(e, "ffprobe"))?;
    if !out.status.success() {
        return Err(fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "ffprobe could not read the file: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        ));
    }
    let p: ProbeOut = serde_json::from_slice(&out.stdout).unwrap_or_default();
    Ok(p.streams)
}

fn missing_ffmpeg(e: std::io::Error, tool: &str) -> ApiError {
    if e.kind() == std::io::ErrorKind::NotFound {
        fail(
            StatusCode::NOT_IMPLEMENTED,
            format!("{tool} is not installed on the computer running katalogd"),
        )
    } else {
        fail(StatusCode::INTERNAL_SERVER_ERROR, format!("{tool}: {e}"))
    }
}

fn is_default(s: &ProbeStream) -> bool {
    s.disposition.get("default").copied().unwrap_or(0) == 1
}

/// The ffmpeg arguments that turn this file into HLS for Safari, or why it
/// cannot be done without re-encoding the picture.
fn plan(streams: &[ProbeStream], input: &FsPath, dir: &FsPath) -> Result<Vec<String>, String> {
    let video: Vec<(usize, &ProbeStream)> = streams
        .iter()
        .filter(|s| s.codec_type == "video")
        .enumerate()
        // Cover pictures in MKV are video streams too.
        .filter(|(_, s)| !matches!(s.codec_name.as_str(), "mjpeg" | "png"))
        .collect();
    let Some(&(vi, v)) = video.first() else {
        return Err("the file has no video".into());
    };
    let mut args: Vec<String> = vec!["-nostdin".into(), "-loglevel".into(), "error".into()];
    args.push("-i".into());
    args.push(input.to_string_lossy().into_owned());
    args.extend(["-map".into(), format!("0:v:{vi}")]);
    match v.codec_name.as_str() {
        "h264" => args.extend(["-c:v".into(), "copy".into()]),
        "hevc" => args.extend(["-c:v".into(), "copy".into(), "-tag:v".into(), "hvc1".into()]),
        other => {
            return Err(format!(
                "the picture is {other}, which Safari cannot play without re-encoding"
            ))
        }
    }
    let audio: Vec<(usize, &ProbeStream)> = streams
        .iter()
        .filter(|s| s.codec_type == "audio")
        .enumerate()
        .collect();
    let pick = audio
        .iter()
        .find(|(_, s)| is_default(s))
        .or_else(|| audio.first());
    if let Some(&(ai, a)) = pick {
        args.extend(["-map".into(), format!("0:a:{ai}")]);
        match a.codec_name.as_str() {
            "aac" | "ac3" | "eac3" => args.extend(["-c:a".into(), "copy".into()]),
            _ => args.extend([
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                "192k".into(),
                "-ac".into(),
                "2".into(),
            ]),
        }
    }
    let seg = dir.join("seg%05d.m4s");
    args.extend(
        [
            "-f",
            "hls",
            "-hls_time",
            "6",
            "-hls_playlist_type",
            "event",
            "-hls_segment_type",
            "fmp4",
            "-hls_fmp4_init_filename",
            "init.mp4",
            "-hls_flags",
            "independent_segments",
            "-hls_segment_filename",
        ]
        .map(String::from),
    );
    args.push(seg.to_string_lossy().into_owned());
    args.push(dir.join("index.m3u8").to_string_lossy().into_owned());
    Ok(args)
}

// ---- the jobs ---------------------------------------------------------------

fn finished(dir: &FsPath) -> bool {
    std::fs::read_to_string(dir.join("index.m3u8"))
        .map(|s| s.contains("#EXT-X-ENDLIST"))
        .unwrap_or(false)
}

/// Starts ffmpeg for this film unless it is running or already done.
/// Returns an error when the film cannot be repackaged.
fn ensure(key: &str, input: &FsPath) -> Result<PathBuf, ApiError> {
    let root = cache_root();
    let dir = root.join(key);
    let mut jobs = JOBS.lock().unwrap();
    if let Some(child) = jobs.get_mut(key) {
        match child.try_wait() {
            Ok(None) => return Ok(dir),
            _ => {
                jobs.remove(key);
            }
        }
    }
    if finished(&dir) {
        // Used again: keep it among the newest.
        let _ = filetime_touch(&dir);
        return Ok(dir);
    }
    let streams = probe(input)?;
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| {
        fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{}: {e}", dir.display()),
        )
    })?;
    let args = plan(&streams, input, &dir).map_err(|m| {
        let _ = std::fs::write(dir.join("unplayable"), &m);
        fail(StatusCode::UNPROCESSABLE_ENTITY, m)
    })?;
    let log = std::fs::File::create(dir.join("ffmpeg.log"))
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let child = Command::new("ffmpeg")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()
        .map_err(|e| missing_ffmpeg(e, "ffmpeg"))?;
    tracing::info!("repackaging {} for Safari", input.display());
    jobs.insert(key.to_string(), child);
    prune(&root, &jobs);
    Ok(dir)
}

fn filetime_touch(dir: &FsPath) -> std::io::Result<()> {
    // A directory's mtime changes when an entry is added or removed.
    let mark = dir.join(".used");
    std::fs::write(&mark, b"")?;
    std::fs::remove_file(mark)
}

/// Keeps the newest few films and the ones being made; the rest go.
fn prune(root: &FsPath, jobs: &HashMap<String, Child>) {
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    let mut dirs: Vec<(std::time::SystemTime, PathBuf, String)> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let t = e.metadata().and_then(|m| m.modified()).ok()?;
            Some((t, e.path(), name))
        })
        .filter(|(_, _, name)| name != "subs" && !jobs.contains_key(name))
        .collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.0));
    for (_, path, _) in dirs.into_iter().skip(KEEP) {
        let _ = std::fs::remove_dir_all(path);
    }
}

fn running(key: &str) -> bool {
    let mut jobs = JOBS.lock().unwrap();
    match jobs.get_mut(key).map(|c| c.try_wait()) {
        Some(Ok(None)) => true,
        Some(_) => {
            jobs.remove(key);
            false
        }
        None => false,
    }
}

fn log_tail(dir: &FsPath) -> String {
    let s = std::fs::read_to_string(dir.join("ffmpeg.log")).unwrap_or_default();
    let line = s.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
    let line: String = line.chars().take(200).collect();
    if line.is_empty() {
        "ffmpeg stopped without saying why".into()
    } else {
        format!("ffmpeg: {line}")
    }
}

/// Serves the playlist, starting ffmpeg the first time and waiting for its
/// first segment. Safari is told to start at the beginning, not the live
/// edge, while the playlist is still growing.
pub async fn playlist(
    State(s): State<Shared>,
    Path((kind, id)): Path<(String, i64)>,
) -> Result<Response, ApiError> {
    let key = key(&kind, id)?;
    let input = PathBuf::from(api::video_path(&s, &kind, id).await?);
    let k = key.clone();
    let dir = tokio::task::spawn_blocking(move || ensure(&k, &input))
        .await
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))??;
    let started = Instant::now();
    let text = loop {
        let text = tokio::fs::read_to_string(dir.join("index.m3u8"))
            .await
            .unwrap_or_default();
        if text.contains("#EXTINF") {
            break text;
        }
        if !running(&key) {
            return Err(fail(StatusCode::INTERNAL_SERVER_ERROR, log_tail(&dir)));
        }
        if started.elapsed() > FIRST_SEGMENT {
            return Err(fail(
                StatusCode::GATEWAY_TIMEOUT,
                "ffmpeg is slow to start; try again in a moment",
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    let text = text.replacen(
        "#EXTM3U\n",
        "#EXTM3U\n#EXT-X-START:TIME-OFFSET=0,PRECISE=YES\n",
        1,
    );
    Ok((
        [
            (header::CONTENT_TYPE, "application/vnd.apple.mpegurl"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        text,
    )
        .into_response())
}

/// One file of a playlist: `init.mp4` or a segment.
pub async fn segment(
    Path((kind, id, name)): Path<(String, i64, String)>,
) -> Result<Response, ApiError> {
    let key = key(&kind, id)?;
    let ok = name == "init.mp4"
        || (name.starts_with("seg")
            && name.ends_with(".m4s")
            && name[3..name.len() - 4].bytes().all(|b| b.is_ascii_digit()));
    if !ok {
        return Err(api::not_found("segment"));
    }
    let path = cache_root().join(key).join(&name);
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|_| api::not_found("segment"))?;
    let mime = if name.ends_with(".mp4") {
        "video/mp4"
    } else {
        "video/iso.segment"
    };
    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "max-age=3600"),
        ],
        Body::from(bytes),
    )
        .into_response())
}

// ---- subtitles inside the file ---------------------------------------------

#[derive(Serialize)]
pub struct Inside {
    /// The index among the file's subtitle streams (ffmpeg's `0:s:n`).
    pub n: usize,
    pub lang: String,
    pub label: String,
}

const TEXT_SUBS: [&str; 5] = ["subrip", "ass", "ssa", "mov_text", "webvtt"];

/// The text subtitles inside a video file; pictures (PGS, DVD) are left out.
pub fn inside_subs(path: &FsPath) -> Vec<Inside> {
    let Ok(streams) = probe(path) else {
        return vec![];
    };
    streams
        .iter()
        .filter(|s| s.codec_type == "subtitle")
        .enumerate()
        .filter(|(_, s)| TEXT_SUBS.contains(&s.codec_name.as_str()))
        .map(|(n, s)| {
            let lang = s.tags.get("language").cloned().unwrap_or_default();
            let title = s.tags.get("title").cloned().unwrap_or_default();
            let label = match (lang.is_empty(), title.is_empty()) {
                (false, false) => format!("{lang} · {title}"),
                (false, true) => lang.clone(),
                (true, false) => title,
                (true, true) => format!("Subtitles {}", n + 1),
            };
            Inside { n, lang, label }
        })
        .collect()
}

/// One text subtitle from inside the file, as WebVTT. ffmpeg has to read the
/// whole file for it, so the answer is kept.
pub async fn inside_sub(
    State(s): State<Shared>,
    Path((kind, id, n)): Path<(String, i64, usize)>,
) -> Result<Response, ApiError> {
    let key = key(&kind, id)?;
    let input = api::video_path(&s, &kind, id).await?;
    let cache = cache_root().join("subs").join(format!("{key}-{n}.vtt"));
    let vtt = match tokio::fs::read(&cache).await {
        Ok(b) => b,
        Err(_) => {
            let out = tokio::process::Command::new("ffmpeg")
                .args(["-nostdin", "-loglevel", "error", "-i", &input])
                .args(["-map", &format!("0:s:{n}"), "-f", "webvtt", "-"])
                .stdin(Stdio::null())
                .output()
                .await
                .map_err(|e| missing_ffmpeg(e, "ffmpeg"))?;
            if !out.status.success() || out.stdout.is_empty() {
                return Err(api::not_found("subtitle"));
            }
            if let Some(p) = cache.parent() {
                let _ = tokio::fs::create_dir_all(p).await;
            }
            let _ = tokio::fs::write(&cache, &out.stdout).await;
            out.stdout
        }
    };
    Ok((
        [(header::CONTENT_TYPE, "text/vtt; charset=utf-8")],
        Body::from(vtt),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(t: &str, c: &str, default: bool) -> ProbeStream {
        ProbeStream {
            codec_type: t.into(),
            codec_name: c.into(),
            disposition: [("default".to_string(), default as i64)].into(),
            ..Default::default()
        }
    }

    fn joined(streams: &[ProbeStream]) -> Result<String, String> {
        plan(streams, FsPath::new("/f.mkv"), FsPath::new("/c")).map(|a| a.join(" "))
    }

    #[test]
    fn hevc_is_copied_and_tagged_for_apple() {
        let a = joined(&[st("video", "hevc", true), st("audio", "aac", true)]).unwrap();
        assert!(a.contains("-map 0:v:0 -c:v copy -tag:v hvc1"), "{a}");
        assert!(a.contains("-map 0:a:0 -c:a copy"), "{a}");
        assert!(a.contains("-hls_segment_type fmp4"), "{a}");
    }

    #[test]
    fn audio_safari_lacks_becomes_aac_and_the_default_track_wins() {
        let a = joined(&[
            st("video", "h264", true),
            st("audio", "aac", false),
            st("audio", "dts", true),
        ])
        .unwrap();
        assert!(a.contains("-map 0:a:1 -c:a aac -b:a 192k -ac 2"), "{a}");
    }

    #[test]
    fn a_cover_picture_is_not_the_film_and_other_codecs_say_why() {
        let a = joined(&[st("video", "mjpeg", false), st("video", "h264", true)]).unwrap();
        assert!(a.contains("-map 0:v:1"), "{a}");
        let e = joined(&[st("video", "av1", true)]).unwrap_err();
        assert!(e.contains("av1"), "{e}");
    }
}
