# Todo

## Now

- Delete the merged branches `comics-blue`, `fetch-command`, `katalogd`, `lyrics-matching`, `playback`

## Next

- Adopt the Streamline design system (design-system@v0.2.0) in place of Satoshi and the hand-written `web/index.css`
- Decide the fate of the Go web catalogue: keep, fold into katalogd, or remove (then `release.yml` and `make dist` go with it)
- Tests: the Go side has none; the Rust side only covers title and artist cleaning in `katalog-core/src/fetch.rs`
- Tag `v0.1.0` so `release.yml` publishes the first binaries

- Playlists and the radio on top of the `plays` table (a chill station, no
  repeat within three plays)
- Window polish: numbers on one baseline, a right-click menu on tracks and
  albums

## Later

- Movies, comics and books in the same window

## Done

- 0.2.0: the desktop window, cropped covers, decade search, a way back from an album
- 0.1.0: katalogd, the katalog command line, the Nix flake, the Go catalogue
