# Katalog

A personal media library for Linux. Music first.

`katalogd` owns the library: it scans `~/Library/Music`, keeps one SQLite file in line
with it, finds album art and lyrics, and serves everything over HTTP so any view can
use it: the desktop window (coming), the `katalog` command line, xmobar, or a phone on
the same network. One real system instead of a daemon and a pile of half-clients.

The plan, the design decisions and the milestones are in the project's plan document.
The older web catalogue (Go, static page) lives on in [docs/web-catalogue.md](docs/web-catalogue.md).

## Run it

Needs Rust 1.85 or newer and the ALSA headers: `alsa-lib` on Arch, `libasound2-dev pkg-config`
on Debian and Ubuntu, or just `nix develop`.

    make build                         # or: cargo build --release
    ./target/release/katalogd          # scans ~/Library/Music, serves http://127.0.0.1:7300
    ./target/release/katalog status    # in another terminal

    katalog albums radiohead
    katalog tracks "paranoid"
    katalog album 12
    katalog lyrics 340
    katalog url 340                    # http://127.0.0.1:7300/stream/340, give it to mpv
    katalog fetch lyrics               # look up missing lyrics now; failures are retried next time
    katalog fetch art --retry          # also look again at albums that had nothing online

The daemon keeps watching the folder: a new rip or a deleted album shows up within seconds.
Nothing is looked up online until you ask: `katalog fetch lyrics` when the wifi is on. Pass
`--auto-fetch` to katalogd if you want it to happen by itself after every scan.

### Playing

    katalog play --album 12            # replace the queue with an album and start
    katalog play 340 341 --start 1     # specific tracks
    katalog add --artist 3             # append everything by an artist
    katalog now                        # track, position bar, format, the current lyric line
    katalog toggle | pause | resume | next | prev | stop
    katalog seek 1:30 | seek +10 | seek -10
    katalog volume 80                  # 100 leaves the samples untouched
    katalog queue | jump 4 | remove 4 | clear
    katalog devices                    # what --device can be set to

Playback goes straight to ALSA. With `--device auto` (the default) katalogd picks the first
hardware output that is not HDMI and opens it as `hw:`, at the file's own sample rate and bit
depth, with nothing in between: a 16-bit FLAC leaves as 16-bit at 44.1 kHz, a 24/96 file as
24 bits at 96 kHz. `katalog now` says `bit-perfect` while that holds. If the card cannot do a
rate, the `plughw:` twin converts for that file and the word disappears. Tracks follow each other
without a gap when they share a format. The card is released when the queue ends or on `stop`,
so other programs can use it. Use `--device default` to go through dmix or PipeWire instead.

Every start is recorded in the `plays` table, and a track counts as finished once half of it
has played. This is what playlists and the radio will be built on.

### Flags

| Flag | Env | Default | Meaning |
| --- | --- | --- | --- |
| `--library` | `KATALOG_LIBRARY` | `~/Library` | Root folder; music is read from `<library>/Music` |
| `--db` | `KATALOG_DB` | `~/.local/share/katalog/katalog.db` | The SQLite file |
| `--bind` | `KATALOG_BIND` | `127.0.0.1:7300` | HTTP address. `0.0.0.0:7300` makes it reachable from your phone |
| `--socket` | `KATALOG_SOCKET` | `$XDG_RUNTIME_DIR/katalogd.sock` | Unix socket for local views |
| `--no-network` | | | Never call MusicBrainz or LRCLIB |
| `--auto-fetch` | | | Look up missing art and lyrics after every scan instead of only on `katalog fetch` |
| `--no-lrc-files` | | | Keep fetched lyrics in the database only, do not write `.lrc` files |
| `--scan-only` | | | Scan once, print the report, exit |
| `--device` | `KATALOG_DEVICE` | `auto` | ALSA output: `auto`, `default`, `hw:1,0`, `hw:CARD=DAC` (`aplay -l` or `katalog devices`) |

## What the scanner reads

- Tags: title, artist, album artist, album, year, track and disc numbers, genre.
  Missing tags fall back to the file and folder names (`01 Airbag.flac` in `OK Computer (1997)`).
- Art: the embedded picture, then `cover.jpg`, `folder.jpg`, `front.png` and friends in the
  album folder, then the Cover Art Archive. Each image is stored once, whatever how many
  tracks carry it.
- Lyrics: the embedded `LYRICS` tag, then `Track.lrc` or `Track.txt` beside the file, then
  LRCLIB (synced when they have them). The lookup tries the exact tags, then the title and album
  with "(Remastered 2009)", "[Live]", "feat." and the like removed, then a search where the hit
  with the same title and a duration within five seconds wins. Fetched lyrics are written back
  as `.lrc` so they stay with your music.
- Fetching is its own step. A lookup that fails (offline, bad answer) is simply tried again next
  time; only a confirmed "nothing found" is remembered, and `--retry` forgets those too.
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
    POST /api/fetch?kind=lyrics|art|all&retry=1   look up missing art and lyrics, answers when done
    GET  /art/{id}                     image bytes, cached forever
    GET  /stream/{track_id}            the audio file, with Range support (seeking works)

    GET  /api/player                   state, track, position, queue ids, format, bit_perfect
    GET  /api/player/queue             the queue as tracks, with the current index
    GET  /api/player/devices           ALSA outputs
    POST /api/player/play              {"tracks":[..]} or {"album":id} or {"artist":id}, "start":n
    POST /api/player/queue             same body, appended
    DELETE /api/player/queue           clear;  DELETE /api/player/queue/{index} removes one
    POST /api/player/{pause|resume|toggle|stop|next|prev}
    POST /api/player/seek?ms=          POST /api/player/volume?v=0..1   POST /api/player/jump?index=

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
