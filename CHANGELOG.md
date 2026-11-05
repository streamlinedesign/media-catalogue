# Changelog

All notable changes, newest first. `make release` turns Unreleased into a version.

## [Unreleased]

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
