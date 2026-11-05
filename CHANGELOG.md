# Changelog

All notable changes, newest first. `make release` turns Unreleased into a version.

## [Unreleased]

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
