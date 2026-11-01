package main

import (
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"regexp"
	"strconv"
	"strings"
	"time"
)

const userAgent = "Katalog/0.1 (personal media catalogue)"

var httpClient = &http.Client{Timeout: 30 * time.Second}

type coverFinder struct {
	tmdbKey      string
	comicVineKey string
	lastMB       time.Time
}

func cmdCovers(args []string) error {
	fl := flag.NewFlagSet("covers", flag.ExitOnError)
	dbPath := fl.String("db", "katalog.db", "SQLite database file")
	tmdbKey := fl.String("tmdb-key", os.Getenv("TMDB_KEY"), "TMDB v3 API key or v4 read token (or set TMDB_KEY)")
	cvKey := fl.String("comicvine-key", os.Getenv("COMICVINE_KEY"), "Comic Vine API key, optional (or set COMICVINE_KEY)")
	force := fl.Bool("force", false, "fetch again even when a cover is already stored")
	only := fl.String("only", "", "limit to one type: movie, tv, album, book or comic")
	fl.Parse(args)

	db, err := openDB(*dbPath, true)
	if err != nil {
		return err
	}
	defer db.Close()

	items, err := loadItems(db)
	if err != nil {
		return err
	}
	have, err := coverMimes(db)
	if err != nil {
		return err
	}
	f := &coverFinder{tmdbKey: *tmdbKey, comicVineKey: *cvKey}
	if f.tmdbKey == "" {
		fmt.Println("No TMDB key given: movies and TV shows will be skipped. Pass -tmdb-key or set TMDB_KEY.")
	}

	var saved int
	var missing []string
	for _, it := range items {
		if *only != "" && it.Type != *only {
			continue
		}
		if _, ok := have[it.ID]; ok && !*force {
			continue
		}
		fmt.Printf("%-5s %s ... ", it.Type, it.Title)
		src := ""
		if it.Cover != nil {
			src = *it.Cover
		} else {
			src, err = f.find(it)
		}
		if err != nil {
			fmt.Println("error:", err)
			missing = append(missing, it.Title)
			err = nil
			continue
		}
		if src == "" {
			fmt.Println("not found")
			missing = append(missing, it.Title)
			continue
		}
		data, mime, err := download(src)
		if err != nil {
			fmt.Println("download failed:", err)
			missing = append(missing, it.Title)
			continue
		}
		_, err = db.Exec(`
			INSERT INTO covers (item_id, mime, data, source_url, fetched_at) VALUES (?, ?, ?, ?, ?)
			ON CONFLICT(item_id) DO UPDATE SET mime=excluded.mime, data=excluded.data,
			  source_url=excluded.source_url, fetched_at=excluded.fetched_at`,
			it.ID, mime, data, src, now())
		if err != nil {
			return err
		}
		saved++
		fmt.Printf("ok (%d KB)\n", len(data)/1024)
	}
	fmt.Printf("\nSaved %d covers into %s.\n", saved, *dbPath)
	if len(missing) > 0 {
		fmt.Printf("No cover for: %s.\nSet \"cover\" to an image URL for those titles and run this again.\n", strings.Join(missing, ", "))
	}
	return nil
}

func (f *coverFinder) find(it Item) (string, error) {
	switch it.Type {
	case "movie", "tv":
		return f.tmdb(it)
	case "album":
		u, err := f.musicBrainz(it)
		if err == nil && u != "" {
			return u, nil
		}
		if err != nil {
			fmt.Printf("(MusicBrainz: %v, trying iTunes) ", err)
		}
		return f.iTunes(it)
	case "book":
		return f.openLibrary(it)
	case "comic":
		if f.comicVineKey != "" {
			u, err := f.comicVine(it)
			if err == nil && u != "" {
				return u, nil
			}
			if err != nil {
				fmt.Printf("(Comic Vine: %v, trying Open Library) ", err)
			}
		}
		return f.openLibrary(it)
	}
	return "", fmt.Errorf("unknown type %q", it.Type)
}

/* ---------------- HTTP helpers ---------------- */

func getJSON(rawURL string, headers map[string]string, out any) error {
	req, err := http.NewRequest("GET", rawURL, nil)
	if err != nil {
		return err
	}
	req.Header.Set("User-Agent", userAgent)
	req.Header.Set("Accept", "application/json")
	for k, v := range headers {
		req.Header.Set(k, v)
	}
	res, err := httpClient.Do(req)
	if err != nil {
		return err
	}
	defer res.Body.Close()
	if res.StatusCode == http.StatusNotFound {
		return errNotFound
	}
	if res.StatusCode != http.StatusOK {
		return fmt.Errorf("%s from %s", res.Status, req.URL.Host)
	}
	return json.NewDecoder(res.Body).Decode(out)
}

var errNotFound = errors.New("not found")

func download(src string) ([]byte, string, error) {
	req, err := http.NewRequest("GET", src, nil)
	if err != nil {
		return nil, "", err
	}
	req.Header.Set("User-Agent", userAgent)
	res, err := httpClient.Do(req)
	if err != nil {
		return nil, "", err
	}
	defer res.Body.Close()
	if res.StatusCode != http.StatusOK {
		return nil, "", fmt.Errorf("%s", res.Status)
	}
	data, err := io.ReadAll(io.LimitReader(res.Body, 20<<20))
	if err != nil {
		return nil, "", err
	}
	mime := http.DetectContentType(data)
	if !strings.HasPrefix(mime, "image/") {
		return nil, "", fmt.Errorf("not an image (%s)", mime)
	}
	return data, mime, nil
}

/* ---------------- TMDB ---------------- */

func (f *coverFinder) tmdb(it Item) (string, error) {
	if f.tmdbKey == "" {
		return "", errors.New("no TMDB key")
	}
	kind := "movie"
	yearParam := "year"
	if it.Type == "tv" {
		kind, yearParam = "tv", "first_air_date_year"
	}
	// v4 read access tokens are long JWTs sent as a bearer token; v3 keys go in the query.
	bearer := len(f.tmdbKey) > 40
	headers := map[string]string{}
	auth := url.Values{}
	if bearer {
		headers["Authorization"] = "Bearer " + f.tmdbKey
	} else {
		auth.Set("api_key", f.tmdbKey)
	}

	var ids struct {
		TMDB int `json:"tmdb"`
	}
	_ = json.Unmarshal(it.IDs, &ids)
	if ids.TMDB > 0 {
		var d struct {
			PosterPath string `json:"poster_path"`
		}
		err := getJSON(fmt.Sprintf("https://api.themoviedb.org/3/%s/%d?%s", kind, ids.TMDB, auth.Encode()), headers, &d)
		if err == nil && d.PosterPath != "" {
			return "https://image.tmdb.org/t/p/w780" + d.PosterPath, nil
		}
		if err != nil && err != errNotFound {
			return "", err
		}
	}

	type result struct {
		PosterPath string `json:"poster_path"`
	}
	search := func(withYear bool) (string, error) {
		q := url.Values{}
		for k, v := range auth {
			q[k] = v
		}
		q.Set("query", it.Title)
		q.Set("include_adult", "false")
		if withYear && it.Year > 0 {
			q.Set(yearParam, strconv.Itoa(it.Year))
		}
		var d struct {
			Results []result `json:"results"`
		}
		if err := getJSON("https://api.themoviedb.org/3/search/"+kind+"?"+q.Encode(), headers, &d); err != nil {
			return "", err
		}
		for _, r := range d.Results {
			if r.PosterPath != "" {
				return "https://image.tmdb.org/t/p/w780" + r.PosterPath, nil
			}
		}
		return "", nil
	}
	if u, err := search(true); err != nil || u != "" {
		return u, err
	}
	return search(false)
}

/* ---------------- Albums ---------------- */

func luceneQuote(s string) string {
	return `"` + strings.NewReplacer(`\`, `\\`, `"`, `\"`).Replace(s) + `"`
}

func (f *coverFinder) musicBrainz(it Item) (string, error) {
	// MusicBrainz asks for at most one request per second.
	if wait := time.Second + 100*time.Millisecond - time.Since(f.lastMB); wait > 0 {
		time.Sleep(wait)
	}
	f.lastMB = time.Now()
	artist := ""
	if len(it.Creators) > 0 {
		artist = it.Creators[0].Name
	}
	q := "releasegroup:" + luceneQuote(it.Title) + " AND primarytype:album"
	if artist != "" {
		q += " AND artist:" + luceneQuote(artist)
	}
	var d struct {
		ReleaseGroups []struct {
			ID string `json:"id"`
		} `json:"release-groups"`
	}
	if err := getJSON("https://musicbrainz.org/ws/2/release-group/?fmt=json&limit=3&query="+url.QueryEscape(q), nil, &d); err != nil {
		return "", err
	}
	for _, rg := range d.ReleaseGroups {
		var art struct {
			Images []struct {
				Front      bool              `json:"front"`
				Image      string            `json:"image"`
				Thumbnails map[string]string `json:"thumbnails"`
			} `json:"images"`
		}
		err := getJSON("https://coverartarchive.org/release-group/"+rg.ID, nil, &art)
		if err == errNotFound {
			continue
		}
		if err != nil {
			return "", err
		}
		for _, img := range art.Images {
			if !img.Front {
				continue
			}
			for _, size := range []string{"1200", "large", "500"} {
				if u := img.Thumbnails[size]; u != "" {
					return strings.Replace(u, "http://", "https://", 1), nil
				}
			}
			return strings.Replace(img.Image, "http://", "https://", 1), nil
		}
	}
	return "", nil
}

var itunesSize = regexp.MustCompile(`/\d+x\d+bb\.`)

func (f *coverFinder) iTunes(it Item) (string, error) {
	term := it.Title
	if len(it.Creators) > 0 {
		term = it.Creators[0].Name + " " + it.Title
	}
	var d struct {
		Results []struct {
			CollectionName string `json:"collectionName"`
			ArtworkURL100  string `json:"artworkUrl100"`
		} `json:"results"`
	}
	if err := getJSON("https://itunes.apple.com/search?entity=album&limit=5&term="+url.QueryEscape(term), nil, &d); err != nil {
		return "", err
	}
	want := strings.ToLower(it.Title)
	best := ""
	for _, r := range d.Results {
		if r.ArtworkURL100 == "" {
			continue
		}
		if best == "" {
			best = r.ArtworkURL100
		}
		if strings.Contains(strings.ToLower(r.CollectionName), want) {
			best = r.ArtworkURL100
			break
		}
	}
	if best == "" {
		return "", nil
	}
	return itunesSize.ReplaceAllString(best, "/1000x1000bb."), nil
}

/* ---------------- Books and comics ---------------- */

func (f *coverFinder) openLibrary(it Item) (string, error) {
	search := func(withAuthor bool) (string, error) {
		q := url.Values{}
		q.Set("title", it.Title)
		q.Set("limit", "5")
		q.Set("fields", "title,cover_i")
		if withAuthor && len(it.Creators) > 0 {
			q.Set("author", it.Creators[0].Name)
		}
		var d struct {
			Docs []struct {
				CoverI int `json:"cover_i"`
			} `json:"docs"`
		}
		if err := getJSON("https://openlibrary.org/search.json?"+q.Encode(), nil, &d); err != nil {
			return "", err
		}
		for _, doc := range d.Docs {
			if doc.CoverI > 0 {
				return fmt.Sprintf("https://covers.openlibrary.org/b/id/%d-L.jpg", doc.CoverI), nil
			}
		}
		return "", nil
	}
	if u, err := search(true); err != nil || u != "" {
		return u, err
	}
	return search(false)
}

func (f *coverFinder) comicVine(it Item) (string, error) {
	q := url.Values{}
	q.Set("api_key", f.comicVineKey)
	q.Set("format", "json")
	q.Set("resources", "volume")
	q.Set("limit", "5")
	q.Set("query", it.Title)
	var d struct {
		Results []struct {
			StartYear string `json:"start_year"`
			Image     struct {
				OriginalURL string `json:"original_url"`
			} `json:"image"`
		} `json:"results"`
	}
	if err := getJSON("https://comicvine.gamespot.com/api/search/?"+q.Encode(), nil, &d); err != nil {
		return "", err
	}
	year := strconv.Itoa(it.Year)
	best := ""
	for _, r := range d.Results {
		if r.Image.OriginalURL == "" {
			continue
		}
		if best == "" {
			best = r.Image.OriginalURL
		}
		if r.StartYear == year {
			return r.Image.OriginalURL, nil
		}
	}
	return best, nil
}
