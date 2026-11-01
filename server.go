package main

import (
	"bytes"
	"database/sql"
	"encoding/json"
	"flag"
	"fmt"
	"io/fs"
	"log"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// siteData builds the two script files the page loads:
// data/catalogue.js (all titles) and data/covers.js (id -> cover path).
func siteData(db *sql.DB) (catalogue, covers []byte, mimes map[string]string, err error) {
	items, err := loadItems(db)
	if err != nil {
		return nil, nil, nil, err
	}
	mimes, err = coverMimes(db)
	if err != nil {
		return nil, nil, nil, err
	}
	paths := map[string]string{}
	for i := range items {
		if mime, ok := mimes[items[i].ID]; ok {
			paths[items[i].ID] = "covers/" + items[i].ID + extFor(mime)
			items[i].Cover = nil // the stored copy wins over the remote URL
		}
	}
	cj, err := json.MarshalIndent(items, "", "  ")
	if err != nil {
		return nil, nil, nil, err
	}
	pj, err := json.MarshalIndent(paths, "", "  ")
	if err != nil {
		return nil, nil, nil, err
	}
	catalogue = append(append([]byte("window.CATALOGUE = "), cj...), ";\n"...)
	covers = append(append([]byte("window.COVERS = "), pj...), ";\n"...)
	return catalogue, covers, mimes, nil
}

func cmdServe(args []string) error {
	fl := flag.NewFlagSet("serve", flag.ExitOnError)
	dbPath := fl.String("db", "katalog.db", "SQLite database file")
	addr := fl.String("addr", ":"+envOr("PORT", "4180"), "address to listen on (or set PORT); production uses -addr 127.0.0.1:4180 behind Caddy")
	fl.Parse(args)

	db, err := openDB(*dbPath, true)
	if err != nil {
		return err
	}
	defer db.Close()

	web, err := fs.Sub(webFS, "web")
	if err != nil {
		return err
	}

	mux := http.NewServeMux()
	mux.HandleFunc("GET /data/catalogue.js", func(w http.ResponseWriter, r *http.Request) {
		cat, _, _, err := siteData(db)
		if err != nil {
			http.Error(w, err.Error(), 500)
			return
		}
		w.Header().Set("Content-Type", "text/javascript; charset=utf-8")
		w.Header().Set("Cache-Control", "no-cache")
		w.Write(cat)
	})
	mux.HandleFunc("GET /data/covers.js", func(w http.ResponseWriter, r *http.Request) {
		_, cov, _, err := siteData(db)
		if err != nil {
			http.Error(w, err.Error(), 500)
			return
		}
		w.Header().Set("Content-Type", "text/javascript; charset=utf-8")
		w.Header().Set("Cache-Control", "no-cache")
		w.Write(cov)
	})
	mux.HandleFunc("GET /api/items", func(w http.ResponseWriter, r *http.Request) {
		items, err := loadItems(db)
		if err != nil {
			http.Error(w, err.Error(), 500)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(items)
	})
	mux.HandleFunc("GET /covers/{file}", func(w http.ResponseWriter, r *http.Request) {
		file := r.PathValue("file")
		id := strings.TrimSuffix(file, filepath.Ext(file))
		var mime, fetched string
		var data []byte
		err := db.QueryRow(`SELECT mime, data, fetched_at FROM covers WHERE item_id = ?`, id).Scan(&mime, &data, &fetched)
		if err == sql.ErrNoRows {
			http.NotFound(w, r)
			return
		} else if err != nil {
			http.Error(w, err.Error(), 500)
			return
		}
		mod, _ := time.Parse(time.RFC3339, fetched)
		w.Header().Set("Content-Type", mime)
		w.Header().Set("Cache-Control", "public, max-age=86400")
		http.ServeContent(w, r, file, mod, bytes.NewReader(data))
	})
	mux.Handle("GET /", http.FileServerFS(web))

	ln, err := net.Listen("tcp", *addr)
	if err != nil {
		return err
	}
	url := "http://" + ln.Addr().String()
	if strings.HasPrefix(*addr, ":") {
		url = "http://localhost" + *addr
	}
	fmt.Printf("Katalog is running at %s  (database: %s, Ctrl+C to stop)\n", url, *dbPath)
	srv := &http.Server{Handler: logRequests(mux), ReadHeaderTimeout: 10 * time.Second}
	return srv.Serve(ln)
}

// envOr returns the environment variable key, or fallback when it is unset or empty.
func envOr(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}

func logRequests(h http.Handler) http.Handler {
	quiet := os.Getenv("KATALOG_QUIET") != ""
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if !quiet && !strings.HasPrefix(r.URL.Path, "/covers/") {
			log.Printf("%s %s", r.Method, r.URL.Path)
		}
		h.ServeHTTP(w, r)
	})
}

func cmdExport(args []string) error {
	fl := flag.NewFlagSet("export", flag.ExitOnError)
	dbPath := fl.String("db", "katalog.db", "SQLite database file")
	out := fl.String("out", "site", "folder to write the static site into")
	fl.Parse(args)

	db, err := openDB(*dbPath, true)
	if err != nil {
		return err
	}
	defer db.Close()

	// 1. the page itself
	err = fs.WalkDir(webFS, "web", func(p string, d fs.DirEntry, err error) error {
		if err != nil || d.IsDir() {
			return err
		}
		data, err := webFS.ReadFile(p)
		if err != nil {
			return err
		}
		return writeFile(filepath.Join(*out, strings.TrimPrefix(p, "web/")), data)
	})
	if err != nil {
		return err
	}

	// 2. data files
	cat, cov, _, err := siteData(db)
	if err != nil {
		return err
	}
	if err := writeFile(filepath.Join(*out, "data", "catalogue.js"), cat); err != nil {
		return err
	}
	if err := writeFile(filepath.Join(*out, "data", "covers.js"), cov); err != nil {
		return err
	}

	// 3. cover images
	rows, err := db.Query(`SELECT item_id, mime, data FROM covers`)
	if err != nil {
		return err
	}
	defer rows.Close()
	n := 0
	for rows.Next() {
		var id, mime string
		var data []byte
		if err := rows.Scan(&id, &mime, &data); err != nil {
			return err
		}
		if err := writeFile(filepath.Join(*out, "covers", id+extFor(mime)), data); err != nil {
			return err
		}
		n++
	}
	fmt.Printf("Exported the catalogue and %d covers to %s/. Open %s/index.html or upload the folder.\n", n, *out, *out)
	return rows.Err()
}

func writeFile(path string, data []byte) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	return os.WriteFile(path, data, 0o644)
}

func cmdImport(args []string) error {
	fl := flag.NewFlagSet("import", flag.ExitOnError)
	dbPath := fl.String("db", "katalog.db", "SQLite database file")
	fl.Usage = func() {
		fmt.Fprintln(os.Stderr, "Usage: katalog import [-db katalog.db] items.json\n\nitems.json is a list of titles in the same shape as the sample data. Titles with an existing id are updated.")
		fl.PrintDefaults()
	}
	fl.Parse(args)
	if fl.NArg() != 1 {
		fl.Usage()
		os.Exit(2)
	}
	raw, err := os.ReadFile(fl.Arg(0))
	if err != nil {
		return err
	}
	var items []Item
	if err := json.Unmarshal(raw, &items); err != nil {
		return fmt.Errorf("%s: %w", fl.Arg(0), err)
	}
	db, err := openDB(*dbPath, false)
	if err != nil {
		return err
	}
	defer db.Close()
	if err := upsertItems(db, items); err != nil {
		return err
	}
	fmt.Printf("Imported %d titles into %s.\n", len(items), *dbPath)
	return nil
}
