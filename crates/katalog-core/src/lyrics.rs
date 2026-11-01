//! Lyrics beside the music: `.lrc` or `.txt` files with the track's name.

use std::path::Path;

/// Looks for `Track.lrc` then `Track.txt` next to `Track.flac`.
pub fn sidecar(audio: &Path) -> Option<String> {
    for ext in ["lrc", "LRC", "txt"] {
        let p = audio.with_extension(ext);
        if let Ok(text) = std::fs::read_to_string(&p) {
            let text = text.trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

/// Synced lyrics carry `[mm:ss.xx]` timestamps at the start of lines.
pub fn is_synced(text: &str) -> bool {
    text.lines().take(20).filter(|l| is_timestamped(l)).count() >= 2
}

fn is_timestamped(line: &str) -> bool {
    let b = line.trim_start().as_bytes();
    b.len() > 6 && b[0] == b'[' && b[1].is_ascii_digit() && b[2].is_ascii_digit() && b[3] == b':'
}

/// Writes lyrics next to the audio file so they stay with the music.
pub fn write_sidecar(audio: &Path, text: &str) -> std::io::Result<()> {
    std::fs::write(audio.with_extension("lrc"), text)
}
