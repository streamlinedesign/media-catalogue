# Katalog

A personal media library for Linux. Music first.

`katalogd` owns the library: it scans `~/Library/Music`, keeps one SQLite file in line
with it, finds album art and lyrics, and serves everything over HTTP so any view can
use it: the desktop window (coming), the `katalog` command line, xmobar, or a phone on
the same network. One real system instead of a daemon and a pile of half-clients.

The plan, the design decisions and the milestones are in the project's plan document.
The older web catalogue (Go, static page) lives on in [docs/web-catalogue.md](docs/web-catalogue.md).

## Run it

Needs Rust 1.85 or newer (or `nix develop`).

    cargo build --release
    ./target/release/katalogd          # scans ~/Library/Music, serves http://127.0.0.1:7300
    ./target/release/katalog status    # in another terminal

    katalog albums radiohead
    katalog tracks "paranoid"
    katalog album 12
    katalog lyrics 340
    katalog url 340                    # http://127.0.0.1:7300/stream/340, give it to mpv

The daemon keeps watching the folder: a new rip or a deleted album shows up within seconds.

### Flags

| Flag | Env | Default | Meaning |
| --- | --- | --- | --- |
| `--library` | `KATALOG_LIBRARY` | `~/Library` | Root folder; music is read from `<library>/Music` |
| `--db` | `KATALOG_DB` | `~/.local/share/katalog/katalog.db` | The SQLite file |
| `--bind` | `KATALOG_BIND` | `127.0.0.1:7300` | HTTP address. `0.0.0.0:7300` makes it reachable from your phone |
| `--socket` | `KATALOG_SOCKET` | `$XDG_RUNTIME_DIR/katalogd.sock` | Unix socket for local views |
| `--no-network` | | | Never call MusicBrainz or LRCLIB |
| `--no-lrc-files` | | | Keep fetched lyrics in the database only, do not write `.lrc` files |
| `--scan-only` | | | Scan once, print the report, exit |

## What the scanner reads

- Tags: title, artist, album artist, album, year, track and disc numbers, genre.
  Missing tags fall back to the file and folder names (`01 Airbag.flac` in `OK Computer (1997)`).
- Art: the embedded picture, then `cover.jpg`, `folder.jpg`, `front.png` and friends in the
  album folder, then the Cover Art Archive. Each image is stored once, whatever how many
  tracks carry it.
- Lyrics: the embedded `LYRICS` tag, then `Track.lrc` or `Track.txt` beside the file, then
  LRCLIB (synced when they have them). Fetched lyrics are written back as `.lrc` so they stay
  with your music.
- Unchanged files (same size and mtime) are skipped, so a rescan of a large library is a
  directory walk, not a re-read.

## HTTP API

    GET  /api/status
    GET  /api/artists?q=
    GET  /api/albums?q=&artist=<id>
    GET  /api/albums/{id}              album with its tracks
    GET  /api/tracks?q=&album=<id>&limit=
    GET  /api/tracks/{id}
    GET  /api/tracks/{id}/lyrics
    POST /api/scan
    GET  /art/{id}                     image bytes, cached forever
    GET  /stream/{track_id}            the audio file, with Range support (seeking works)

All answers are JSON except `/art` and `/stream`. CORS is open, so a web view can talk to it directly.

## Layout

    crates/katalog-core   library, tags, art, lyrics (no network loop, no server)
    crates/katalogd       the daemon: scan, watch, fetch, serve
    crates/katalog        the command line view
    flake.nix             dev shell, package, and a user service for NixOS or home-manager
    web/, *.go            the earlier web catalogue, see docs/web-catalogue.md

## Database

| Table | Holds |
| --- | --- |
| `artists`, `albums`, `tracks` | The library. `tracks.path` is unique; `albums` are keyed by artist, title and folder |
| `art` | Images, deduplicated by hash |
| `lyrics` | One text per track, synced or plain, with its source |
| `plays`, `track_tags`, `playlists` | Ready for the player and radio milestones |
