# Todo

## Now

- Run make configure-repo once
- Review and merge PR #6 (`pause-and-lyrics-search`: play after pause, pick lyrics by hand)
- Delete the merged branches `comics-blue`, `fetch-command`, `katalogd`, `lyrics-matching`, `playback`

## Next

- Adopt the Streamline design system (design-system@v0.2.0) in place of Satoshi and the hand-written `web/index.css`
- Decide the fate of the Go web catalogue: keep, fold into katalogd, or remove (then `release.yml` and `make dist` go with it)
- Tests: the Go side has none; the Rust side only covers title and artist cleaning in `katalog-core/src/fetch.rs`
- Tag `v0.1.0` so `release.yml` publishes the first binaries

## Later

- The desktop window the README promises
- Playlists and the radio on top of the `plays` table

## Done

- 0.1.0: katalogd, the katalog command line, the Nix flake, the Go catalogue
