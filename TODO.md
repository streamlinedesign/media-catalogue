# Todo

## Now

- Delete the merged branches `comics-blue`, `fetch-command`, `katalogd`, `lyrics-matching`, `playback`

## Next

- Adopt the Streamline design system (design-system@v0.2.0) in place of Satoshi and the hand-written `web/index.css`
- Decide the fate of the Go web catalogue: keep, fold into katalogd, or remove (then `release.yml` and `make dist` go with it)
- Tests: the Go side has none; the Rust side only covers title and artist cleaning in `katalog-core/src/fetch.rs`
- Tag `v0.1.0` so `release.yml` publishes the first binaries

- Window polish: numbers on one baseline, a right-click menu on tracks and
  albums
- Reorder a playlist by dragging; `katalog playlist-move`
- Stations from tags once `track_tags` is in use

- CRT: screen curvature and a little bloom, as a shader when the GPU renderer is in use

## Later

- Reader: search inside a book; EPUB footnotes as pop-ups

## Done

- 0.14.3: page turn buttons in the reader
- 0.14.2: touch turns the page
- 0.14.1: rotate turns back, tap halves turn pages, arrows scroll, big RAR comics load fast
- 0.14.0: books and comics, and a reader with highlights, notes, bookmarks, dark PDFs and rotate
- 0.13.5: fetch missing subtitles from opensubtitles.com
- 0.13.4: remove wrong lyrics
- 0.13.3: films play with no channel on; lengths from mpv when ffprobe is missing
- 0.13.2: playback and the TV share the card without the daemon getting lost; radio off stops
- 0.13.1: TV turns back on, TV sound works, lighter CRT
- 0.13.0: four pages: Music, Screen, Catalogue, Settings
- 0.12.1: the TV and the music share the sound card; TV off stays off; stronger CRT
- 0.12.0: tonight's film
- 0.11.1: extras left out of films and shows; TMDB after every scan
- 0.11.0: the television on Home, three channels
- 0.10.0: watching films and episodes in mpv, English subtitles by default
- 0.9.0: films and TV in the catalogue
- 0.8.0: playlists can be kept off the dial
- 0.7.0: the CRT overlay
- 0.6.0: a stream address per station
- 0.5.0: stations, always on, joined live
- 0.4.0: the Home tab, the spectrum feed, album picks
- 0.3.0: playlists, the radio, the loop guard, add-to-playlist on every song
- 0.2.0: the desktop window, cropped covers, decade search, a way back from an album
- 0.1.0: katalogd, the katalog command line, the Nix flake, the Go catalogue
