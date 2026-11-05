//! Katalog core: the music library on disk, read into SQLite, with art and lyrics.
//!
//! Nothing here plays audio or draws a window. The daemon and the CLI build on this.

pub mod db;
pub mod fetch;
pub mod lyrics;
pub mod scan;
pub mod stations;

pub use db::{Album, Artist, Lyrics, Stats, Track};
pub use scan::{ScanReport, Scanner};

/// File extensions the scanner treats as music.
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "flac", "mp3", "ogg", "oga", "opus", "m4a", "aac", "wav", "aiff", "aif", "wv", "ape",
];

pub fn is_audio_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}
