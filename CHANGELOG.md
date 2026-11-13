# Changelog

All notable changes, newest first. `make release` turns Unreleased into a version.

## [Unreleased]

- Cover art: when MusicBrainz knows no album by the title as tagged, the lookup
  tries again without what rips add to it ("CD", "CD1", "Disc 2", "(Disc 02)",
  "[flac]"), so "At the Edge of Time CD" finds its cover. `katalog fetch art
  --retry` looks again for albums that had nothing.
- Phone page: the film or episode comes first, then the poster, title and
  the rest.
- MKV films and episodes play in Safari on the iPhone: the phone page asks
  for HLS, and katalogd repackages the file with ffmpeg on the fly,
  copying the picture (H.264, HEVC as `hvc1`) and the sound when Safari
  knows it, else stereo AAC. The last three stay in `~/.cache/katalog/hls`.
  Text subtitles inside a file now show in the browser too, as WebVTT.
  New: `GET /web/hls/{kind}/{id}/index.m3u8`, `GET /web/isub/{kind}/{id}/{n}`.
- Phone page: Open in VLC works on Android (an intent naming VLC; the
  vlc:// address got mangled and VLC gave a playback error) and on iOS
  (VLC's x-callback link). Copy link shows the file's address, sign-in
  included, for VLC's Open network stream or any other player.
- The phone page: with `KATALOG_WEB=0.0.0.0:4180` and `KATALOG_WEB_PASSWORD`,
  katalogd serves a page on the Wi-Fi for music (albums, playlists, a
  player with lock screen controls), films and shows (in the browser when
  its codecs allow, else Download or Open in VLC, with subtitles beside
  the file as WebVTT) and book and comic downloads. It is its own
  listener, read-only, behind the password, with a cookie that lasts a
  year. `make dev` runs it on 0.0.0.0:4180 and the API on 127.0.0.1:4181.
  New: `GET /books/{id}/file`.
- The earlier Go web catalogue is gone: `*.go`, `web/`, `seed.json` and
  `docs/web-catalogue.md`. `make dev` now runs katalogd from source on
  127.0.0.1:4180, `make test`, `build` and `lint` are Rust only,
  `db-reset` and `db-seed` say they are not used, and `make dist` packs
  the three binaries. `release.yml` builds those on a `v*` tag. Go left
  the Nix dev shell.
- Genre playlists, made by themselves: after a scan, every genre with at
  least 8 tracks gets a playlist ("Rock; New Wave" counts for both), in
  artist and album order, refilled after each scan and dropped when the
  genre goes. They say "genre" beside their count and start off the radio
  dial; switch one on in Settings to make it a station. A hand-made
  playlist of the same name keeps it; the made one is "Rock (genre)".
- Film controls: under the set, while a film or episode you asked for
  plays, ← 10, Pause and 30 →. On Screen or fullscreen the arrow keys wind
  10 seconds and space pauses the film instead of the music.
- Where you left off, for films and shows, apart from the TV channels.
  While a film or episode you asked for plays, the window notes where it
  is every five seconds; Play and Resume start from there. Near the end
  (the last 5 %, or two minutes of anything longer than 20) it counts as
  watched and is let go, and an episode leaves the next one of its show
  waiting. Screen lists them under Tonight's film, with Resume and ×.
  `GET /api/left-off`, `PUT`/`DELETE /api/{movies|episodes}/{id}/progress`.
- A channel key while a film plays tunes the channel again (it was
  ignored), and a channel item that simply ends within seconds is no
  longer taken for one that failed to start.
- Search inside a book: a Search tab beside Contents and Marks (or `/`)
  finds words in an EPUB or a PDF, ignoring case, and lists each place
  with the words around it; a click goes there. katalogd answers
  `GET /api/books/{id}/search?q=`.
- Page turn buttons: a ← and a → at the two ends of the reader's foot,
  for books, PDFs and comics, wide enough for a finger.
- Touch turns the page. Taps are now read from the window itself rather
  than by the page, which never saw them; a tap on the right half of a
  comic or PDF goes on, the left half back, and a drag still scrolls.
- Reader fixes. Rotate now has a way back: Turn back and Other way in
  the bar, `r` to turn back, and the screen comes back when the window
  quits; the touchscreen and pen are mapped to the turned screen so taps
  land where the finger is. A click or tap on the right half of a page
  turns on, the left half back, with touch as well as the mouse. Up and
  down scroll a book instead of turning it. Big RAR comics open much
  faster: katalogd unpacks one once instead of for every page, and each
  page shows small at once while the full one loads.
- Books and comics in katalogd: it reads `Books/` (EPUB, PDF) and
  `Comics/` (CBR, CBZ, PDF, folders of pages) next to `Music/`, with
  titles, authors, series, issue numbers and covers. It serves EPUB
  chapters as paragraphs, PDF and comic pages as images (dark pages on
  request), the words on a PDF page with their boxes, and keeps where
  you are, bookmarks, and highlights with notes.
- The window reads them: a Read page with what you are reading, your
  highlights and the comics on the pile, and a reader for EPUB (set in
  Literata, with size, line height and width of your choosing), PDF (dark
  pages, fit width, fit page, two pages) and comics (spreads, right to
  left). Highlights in three colours with notes, as many bookmarks as you
  like, a marks panel that copies them all as Markdown, and a rotate key
  that turns the screen for a foldable. Books and comics also have their
  own sections in the catalogue.
- Subtitles from opensubtitles.com. With `OPENSUBTITLES_KEY` set, Find
  subtitles on a film's page (or "subs" beside an episode) lists what
  there is in `KATALOG_SUB_LANGS`, the ones made for this very file
  first, and a pick is saved beside the video as `Film.hr.srt`. Old
  Windows code pages (1250 for Croatian and so on) are turned into UTF-8.
- Lyrics can be removed: Remove beside the lyrics takes them away and
  goes back to the search, to pick better ones or leave the song without.
  Removed lyrics stay away: the online lookup skips the song and a rescan
  does not bring back an embedded tag or a `.lrc` (files are left as
  they are). Picking lyrics by hand lifts it. `DELETE
  /api/tracks/{id}/lyrics` does the same.
- Films and episodes play on the set even when no channel has anything
  on: a channel answer arriving after you pressed play no longer stops
  the film. Without ffprobe (part of ffmpeg), katalogd asks mpv for a
  film's length, so the channels fill; the next scan fills in the ones
  that had none.
- Playback no longer gets lost when the TV has the sound card. katalogd
  used to freeze for two seconds per track waiting for it, then skip to
  the next track or play through another device. Now it waits without
  freezing, shows the track as playing so the window hands the card back,
  and pauses with a note if the card stays busy. Radio off stops the
  music. The window sends mpv its commands from a thread of its own, so
  it no longer stalls on a slow or dead mpv. A film or channel that will
  not start says why under the set (from mpv's log in
  `~/.cache/katalog/tv.log`) and the set stops retrying for 30 seconds
  instead of starting mpv over and over. mpv decodes on the graphics
  card when it can.
- The TV turns back on after TURN OFF, and the channel keys bring it back
  too. Its sound now reaches you: asking for sound retries until mpv is
  listening, AUDIO on a silent set turns sound on, and mpv plays on the
  same card as katalogd. A lighter CRT.
- Four pages instead of eight tabs. Music is the old Home without the
  television. Screen is a home for films and shows: the set big, its keys,
  a guide to the three channels, tonight's film and what is new.
  Catalogue puts albums, films and shows on one page, each on its colour,
  with an index that stays put; search covers all three. Settings has a
  page for music (playlists, the queue, the library, the CRT, Scan now and
  Fetch) and one for films and shows (folders, subtitles, posters, the set
  and its channels). The Artists tab is gone; an artist's name on an album
  still lists their albums. Posters no longer draw past the bottom of a
  scrolled page.
- The TV no longer takes the sound card from the music. A silent set has
  no audio track at all; it gets sound only when it goes fullscreen or
  plays a film you asked for, after the music is paused, and gives it
  back the moment music is asked for. katalogd lets go of the card while
  paused, and waits a moment for it when it is busy. TURN OFF folds the
  set to one line and stays off until TURN ON. The CRT shows on black:
  darker scanlines with a faint lit line between them.
- Tonight's film at the top of the Movies tab: poster, title, release
  date, length and genres, a taste of the plot, Play now, More and
  Another. One a day, the same all day.
- Extras are no longer films or episodes. A folder that holds a film
  holds only that film, so videos in folders below it are extras; in a
  show only S01E02 files and `Season` folders count; Plex and Jellyfin
  extras names are skipped everywhere. With a TMDB key, katalogd looks
  up new films and shows after every scan by itself.
- A television on Home: three channels always on (Films, Series, and
  Everything), scheduled like the radio. mpv draws inside the set on X11,
  muted until a click makes it fullscreen with sound; Play puts a film on
  the set. SUB, AUDIO and POWER keys; `KATALOG_TV=0` starts it off.
  `/api/channels` and `/api/channels/{n}/watch`.
- Watching: Play on a film, or a click on an episode, opens it in mpv
  with English subtitles on when there are any and none otherwise; music
  pauses. `katalog watch <film>` and `katalog watch <show> s01e02` do the
  same from the terminal. `/api/movies/{id}/watch` hands out the mpv
  arguments, and `/video/...` serves the files to another machine.
- Films and TV: katalogd scans `Movies/` and `TV-Shows/` beside `Music/`,
  reading titles, years and S01E02 from the names, lengths and subtitle
  languages from ffprobe, subtitle files and folder posters. With
  `TMDB_KEY` set, `katalog fetch video` adds plots, dates, genres and
  posters. The window has a Movies tab on black and a TV tab on wine.
  `katalog movies`, `shows`, `show`; `/api/movies`, `/api/shows`.
- A playlist can be taken off the radio dial: the "On the dial" chip on
  its page, `katalog playlist-station <name> on|off`, or
  `PUT /api/playlists/{id}` with `{"station": false}`. Playlists from
  before stay on.
- Window: a CRT overlay over the whole window, scanlines and a soft
  vignette, drawn once per size and taking no input. `c` turns it off and
  on; `KATALOG_CRT=0` starts without it.
- Stations as streams: `GET /stations/<name>.wav` plays a station live as
  an endless 44.1 kHz 16-bit stereo WAV that mpv, VLC or a phone can open
  (files at other rates are resampled for the stream only). A thread per
  listener, nothing when nobody listens. `katalog stations` prints each
  station's address.
- Volume now turns the sound card's own mixer control, the one alsamixer
  shows, with alsamixer's numbers and curve; playback stays bit-perfect at
  every volume. Cards without a control still scale the samples. The player
  status says which (`hw_volume`).
- Window: holding `+` or `−` no longer snaps the volume back; a status that
  predates the change is not allowed to undo it.
- Home: the cover is shown as it is, not tinted green.
- Spectrum: each analysis is stamped with the moment the card plays it and
  the window asks 30 times a second, so the bars no longer trail or stutter
  behind the sound.
- Stations: every playlist, and the whole library, is a station that is
  always on. A station is a schedule (a shuffled order that changes every
  time round, the same artist kept apart, laid end to end from a fixed
  start), so tuning in joins the song that is playing part way through, and
  nothing runs until someone listens. The radio now plays stations: this
  replaces the random picks with a three-day rest.
- `katalog stations` and `GET /api/stations`: what is on every station now.
- Window: the receiver on Home shows what is on the tuned station, on or off.
- Window: a Home tab, the new first screen, after the Spectra-69 "Minimal"
  design: green on black in IBM Plex Mono, the cover tinted to the screen,
  a plain spectrum, time and keys, up next / playlists / for you, five lines
  of lyrics, and a receiver whose stations are the playlists (tuning and
  power drive the radio).
- katalogd: `GET /api/player/levels`, a 32-band spectrum of what is heard
  right now (computed on the way to the card, released after the card's
  delay so it keeps time with the sound).
- katalogd: `GET /api/albums/{id}/picks`, four albums to go with one: same
  genre, same artist (or another of the genre), a wildcard, same century.
- Window: volume − and + in the now-playing bar, and the `-` and `+` keys,
  in steps of five; at 100 playback is bit-perfect again.
- Window: when the playing track has no lyrics, the Lyrics tab searches
  LRCLIB with the title (editable, with or without the artist) and saves the
  hit you click. No terminal needed.
- Playlists: `katalog playlist-new|playlist-add|playlist|playlists|playlist-remove|
  playlist-rename|playlist-delete`, `play --playlist`, the HTTP API under
  `/api/playlists`, and in the window a Playlists tab, a page per playlist
  and a `+` on every song.
- The radio: `katalog radio [playlist]` keeps the queue one random, rested
  track ahead, never the same artist twice in a row; `r` in the window.
- The loop guard: a track plays at most three times in a row, then the
  player moves on and says so.
- `plays.source` records manual, playlist or radio.
- Window: the Satoshi files now carry the family name "Satoshi", so running
  text is set in Satoshi Medium (it was falling back to another face, and
  "č" and friends showed as boxes).
- Window: smaller type throughout (about 15 per cent), tighter grid rows.
- `katalog-window`: the desktop window (iced, no toolkit): album grid, artists,
  queue, synced lyrics, a now-playing card with transport and seek, keyboard
  shortcuts.
- Window fixes: covers are cropped square on the server (`/art/{id}?crop=1`)
  so no cover draws over its neighbours; the search box understands decades
  and ranges ("197", "70s", "1975-1980"); an album page goes back with the
  Albums tab, Backspace, the mouse back button or the "All albums" chip.
- House conventions: `CLAUDE.md`, `VERSION`, `TODO.md`, `.env.example`, CI and
  auto-PR workflows, the shared Makefile targets; the Go catalogue listens on
  `PORT` (4180) instead of 8080.
- `make install` puts the binaries in `~/.local/bin` and runs katalogd as a
  systemd user service (`deploy/katalogd.service`, settings in
  `~/.config/katalog/env`); `make uninstall` removes it.

## [0.1.0] - 2026-11-01

- Everything so far: `katalogd` (scan, watch, art, lyrics, ALSA playback, HTTP
  API), the `katalog` command line, the Nix flake, and the earlier Go web
  catalogue with its sample titles and GitHub release workflow.
