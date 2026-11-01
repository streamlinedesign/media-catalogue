package main

import (
	"database/sql"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	_ "modernc.org/sqlite" // pure Go SQLite: no cgo, cross-compiles anywhere
)

type Creator struct {
	Role string `json:"role"`
	Name string `json:"name"`
}

// Item is one title. The JSON shape is the same one the web page reads.
type Item struct {
	ID       string          `json:"id"`
	Type     string          `json:"type"`
	Title    string          `json:"title"`
	Year     int             `json:"year"`
	Creators []Creator       `json:"creators"`
	Genres   []string        `json:"genres"`
	Meta     json.RawMessage `json:"meta,omitempty"` // kept raw so key order survives
	Synopsis string          `json:"synopsis"`
	Cover    *string         `json:"cover"` // external image URL override, or null
	IDs      json.RawMessage `json:"ids,omitempty"`
}

var validTypes = map[string]bool{"movie": true, "tv": true, "album": true, "book": true, "comic": true}

const schema = `
CREATE TABLE IF NOT EXISTS items (
  id         TEXT PRIMARY KEY,
  type       TEXT NOT NULL CHECK (type IN ('movie','tv','album','book','comic')),
  title      TEXT NOT NULL,
  year       INTEGER,
  synopsis   TEXT NOT NULL DEFAULT '',
  meta       TEXT NOT NULL DEFAULT '{}',
  ids        TEXT NOT NULL DEFAULT '{}',
  cover_url  TEXT,
  path       TEXT,                 -- reserved for the file system link
  added_at   TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS items_type_year ON items(type, year);

CREATE TABLE IF NOT EXISTS creators (
  item_id  TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  position INTEGER NOT NULL,
  role     TEXT NOT NULL DEFAULT '',
  name     TEXT NOT NULL,
  PRIMARY KEY (item_id, position)
);
CREATE INDEX IF NOT EXISTS creators_name ON creators(name);

CREATE TABLE IF NOT EXISTS genres (
  item_id  TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  position INTEGER NOT NULL,
  genre    TEXT NOT NULL,
  PRIMARY KEY (item_id, position)
);
CREATE INDEX IF NOT EXISTS genres_genre ON genres(genre);

CREATE TABLE IF NOT EXISTS covers (
  item_id    TEXT PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
  mime       TEXT NOT NULL,
  data       BLOB NOT NULL,
  source_url TEXT NOT NULL,
  fetched_at TEXT NOT NULL
);
`

// openDB opens (or creates) the database. With seed, an empty database gets the sample titles.
func openDB(path string, seed bool) (*sql.DB, error) {
	dsn := "file:" + path + "?_pragma=foreign_keys(1)&_pragma=journal_mode(WAL)&_pragma=busy_timeout(5000)"
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(1) // one writer keeps SQLite simple and safe
	if _, err := db.Exec(schema); err != nil {
		db.Close()
		return nil, fmt.Errorf("create schema: %w", err)
	}
	var n int
	if err := db.QueryRow(`SELECT COUNT(*) FROM items`).Scan(&n); err != nil {
		db.Close()
		return nil, err
	}
	if n == 0 && seed {
		var sample []Item
		if err := json.Unmarshal(seedJSON, &sample); err != nil {
			db.Close()
			return nil, fmt.Errorf("read sample data: %w", err)
		}
		if err := upsertItems(db, sample); err != nil {
			db.Close()
			return nil, fmt.Errorf("add sample data: %w", err)
		}
		fmt.Printf("Created %s with %d sample titles.\n", path, len(sample))
	}
	return db, nil
}

func now() string { return time.Now().UTC().Format(time.RFC3339) }

func rawOr(r json.RawMessage, def string) string {
	s := strings.TrimSpace(string(r))
	if s == "" || s == "null" {
		return def
	}
	return s
}

func validate(it Item) error {
	switch {
	case strings.TrimSpace(it.ID) == "":
		return fmt.Errorf("a title is missing its id")
	case !validTypes[it.Type]:
		return fmt.Errorf("%s: type must be movie, tv, album, book or comic (got %q)", it.ID, it.Type)
	case strings.TrimSpace(it.Title) == "":
		return fmt.Errorf("%s: title is empty", it.ID)
	}
	return nil
}

func upsertItems(db *sql.DB, items []Item) error {
	tx, err := db.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()
	ts := now()
	for _, it := range items {
		if err := validate(it); err != nil {
			return err
		}
		_, err := tx.Exec(`
			INSERT INTO items (id, type, title, year, synopsis, meta, ids, cover_url, added_at, updated_at)
			VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
			ON CONFLICT(id) DO UPDATE SET
			  type=excluded.type, title=excluded.title, year=excluded.year, synopsis=excluded.synopsis,
			  meta=excluded.meta, ids=excluded.ids, cover_url=excluded.cover_url, updated_at=excluded.updated_at`,
			it.ID, it.Type, it.Title, it.Year, it.Synopsis, rawOr(it.Meta, "{}"), rawOr(it.IDs, "{}"), it.Cover, ts, ts)
		if err != nil {
			return fmt.Errorf("%s: %w", it.ID, err)
		}
		if _, err := tx.Exec(`DELETE FROM creators WHERE item_id = ?`, it.ID); err != nil {
			return err
		}
		if _, err := tx.Exec(`DELETE FROM genres WHERE item_id = ?`, it.ID); err != nil {
			return err
		}
		for i, c := range it.Creators {
			if _, err := tx.Exec(`INSERT INTO creators (item_id, position, role, name) VALUES (?, ?, ?, ?)`, it.ID, i, c.Role, c.Name); err != nil {
				return err
			}
		}
		for i, g := range it.Genres {
			if _, err := tx.Exec(`INSERT INTO genres (item_id, position, genre) VALUES (?, ?, ?)`, it.ID, i, g); err != nil {
				return err
			}
		}
	}
	return tx.Commit()
}

func loadItems(db *sql.DB) ([]Item, error) {
	rows, err := db.Query(`SELECT id, type, title, COALESCE(year, 0), synopsis, meta, ids, cover_url FROM items ORDER BY type, title`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var items []Item
	index := map[string]int{}
	for rows.Next() {
		var it Item
		var meta, ids string
		var cover sql.NullString
		if err := rows.Scan(&it.ID, &it.Type, &it.Title, &it.Year, &it.Synopsis, &meta, &ids, &cover); err != nil {
			return nil, err
		}
		it.Meta, it.IDs = json.RawMessage(meta), json.RawMessage(ids)
		if cover.Valid && cover.String != "" {
			s := cover.String
			it.Cover = &s
		}
		it.Creators, it.Genres = []Creator{}, []string{}
		index[it.ID] = len(items)
		items = append(items, it)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}

	crows, err := db.Query(`SELECT item_id, role, name FROM creators ORDER BY item_id, position`)
	if err != nil {
		return nil, err
	}
	defer crows.Close()
	for crows.Next() {
		var id string
		var c Creator
		if err := crows.Scan(&id, &c.Role, &c.Name); err != nil {
			return nil, err
		}
		if i, ok := index[id]; ok {
			items[i].Creators = append(items[i].Creators, c)
		}
	}

	grows, err := db.Query(`SELECT item_id, genre FROM genres ORDER BY item_id, position`)
	if err != nil {
		return nil, err
	}
	defer grows.Close()
	for grows.Next() {
		var id, g string
		if err := grows.Scan(&id, &g); err != nil {
			return nil, err
		}
		if i, ok := index[id]; ok {
			items[i].Genres = append(items[i].Genres, g)
		}
	}
	return items, nil
}

// coverMimes maps item id to the mime type of its stored cover.
func coverMimes(db *sql.DB) (map[string]string, error) {
	rows, err := db.Query(`SELECT item_id, mime FROM covers`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := map[string]string{}
	for rows.Next() {
		var id, mime string
		if err := rows.Scan(&id, &mime); err != nil {
			return nil, err
		}
		out[id] = mime
	}
	return out, rows.Err()
}

func extFor(mime string) string {
	switch mime {
	case "image/png":
		return ".png"
	case "image/webp":
		return ".webp"
	case "image/gif":
		return ".gif"
	default:
		return ".jpg"
	}
}
