# Changelog

All notable changes, newest first. `make release` turns Unreleased into a version.

## [Unreleased]

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
