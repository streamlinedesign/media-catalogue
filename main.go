// Katalog: a personal media catalogue in one binary.
//
//	katalog serve              serve the catalogue at http://localhost:8080
//	katalog covers             fetch original posters and cover art into the database
//	katalog import items.json  add or update titles from a JSON file
//	katalog export -out site   write a static copy (for GitHub Pages)
//
// Everything lives in one SQLite file (katalog.db by default).
package main

import (
	"embed"
	"fmt"
	"os"
)

//go:embed web
var webFS embed.FS

//go:embed seed.json
var seedJSON []byte

const usageText = `Katalog: personal media catalogue

Usage:
  katalog serve   [-db katalog.db] [-addr :8080]
  katalog covers  [-db katalog.db] [-tmdb-key KEY] [-comicvine-key KEY] [-force] [-only TYPE]
  katalog import  [-db katalog.db] items.json
  katalog export  [-db katalog.db] [-out site]

The database is created and filled with sample titles on first use.
TMDB_KEY and COMICVINE_KEY environment variables work in place of the key flags.
Run "katalog <command> -h" for the flags of one command.
`

func main() {
	if len(os.Args) < 2 {
		fmt.Fprint(os.Stderr, usageText)
		os.Exit(2)
	}
	cmd, args := os.Args[1], os.Args[2:]
	var err error
	switch cmd {
	case "serve":
		err = cmdServe(args)
	case "covers":
		err = cmdCovers(args)
	case "import":
		err = cmdImport(args)
	case "export":
		err = cmdExport(args)
	case "help", "-h", "--help":
		fmt.Print(usageText)
		return
	default:
		fmt.Fprintf(os.Stderr, "unknown command %q\n\n%s", cmd, usageText)
		os.Exit(2)
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, "error:", err)
		os.Exit(1)
	}
}
