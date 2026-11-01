# Katalog

A personal catalogue of movies, TV shows, albums, books and comics in one binary.
The web page is built into the binary and everything else lives in one SQLite file.
There's no Node, no runtime to install and no other files to ship.

## Run it

    ./katalog serve

Open http://localhost:4180. On first run, `katalog.db` is created next to you with 39 sample titles.
The port comes from `PORT` (4180 by default, see PORTS.md in `streamlinedesign/template`); `-addr 127.0.0.1:4180` keeps it local, and `-db path/to/file.db` uses another database.

Build it on Linux with `make katalog` (needs Go 1.24+), or download a binary from the
[Releases](https://github.com/streamlinedesign/media-catalogue/releases) page once a version is tagged:

    chmod +x katalog-linux-amd64 && ./katalog-linux-amd64 serve

`make dist` builds every platform into `dist/`. Pushing a tag such as `v0.1.0` makes GitHub
build the same binaries and attach them to a release.

## Original posters and covers

    ./katalog covers -tmdb-key YOUR_TMDB_KEY

This looks up every title and stores the image itself in the database, so the catalogue works offline afterwards.

| Type        | Source |
|-------------|--------|
| Movies, TV  | TMDB (a v3 API key or a v4 read token) |
| Albums      | MusicBrainz + Cover Art Archive, then the iTunes Search API as a fallback |
| Books       | Open Library |
| Comics      | Comic Vine with `-comicvine-key`, otherwise Open Library |

Titles that already have a cover are skipped. Use `-force` to fetch again, or `-only album` to do one type.
If a lookup picks the wrong image, set `"cover": "https://..."` on that title, import it, and run `covers -force -only <type>`.

## Add your own titles

    ./katalog import my-titles.json

The file is a JSON list in the same shape as `seed.json`. A title with an existing `id` is updated.
Importing into a new database skips the sample titles: `./katalog import -db mine.db my-titles.json`.

## Publish a static copy (GitHub Pages)

    ./katalog export -out site

This writes a plain folder with the page, the data and every cover image. Upload it anywhere.

## Database

| Table      | What it holds |
|------------|---------------|
| `items`    | One row per title: type, title, year, synopsis, `meta` and `ids` as JSON, plus `path` reserved for the file system link |
| `creators` | Directors, artists, authors, in order |
| `genres`   | Genres, in order |
| `covers`   | The cover image bytes and where they came from |

`GET /api/items` returns the whole catalogue as JSON.

## Build from source

Needs Go 1.24 or newer. The SQLite driver is pure Go, so no C compiler is needed and every platform cross-compiles:

    make katalog  # ./katalog for this machine
    make dist     # Linux, macOS and Windows binaries in dist/

## Layout

    main.go      commands
    db.go        schema, reading and writing titles
    server.go    serve, export, import
    covers.go    poster and cover lookups
    seed.json    sample titles
    web/         the page (index.html, index.css, app.js, font), built into the binary
