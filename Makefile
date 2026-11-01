SHELL := bash
.DEFAULT_GOAL := help

# ---- Project ----------------------------------------------------------------
# media-catalogue: the Rust music daemon katalogd and katalog CLI (crates/),
# plus the earlier Go web catalogue (*.go, web/). Ports from PORTS.md in
# streamlinedesign/template; .env overrides. PORT is the Go catalogue; the Rust
# daemon has its own --bind (KATALOG_BIND).
NAME     ?= katalog
PORT     ?= 4180
DROPLET  ?=
DB       ?= katalog.db
# -----------------------------------------------------------------------------

-include .env
export

VERSION := $(shell cat VERSION 2>/dev/null || echo 0.0.0)
LDFLAGS := -s -w
PLATFORMS := linux/amd64 linux/arm64 darwin/amd64 darwin/arm64 windows/amd64

help: ## List targets
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk -F ':.*## ' '{printf "  %-14s %s\n", $$1, $$2}'

dev: ## Go catalogue on PORT (open http://localhost:4180)
	@echo "On your phone: http://$$(hostname -I 2>/dev/null | cut -d' ' -f1 || ipconfig getifaddr en0):$(PORT)"
	go run . serve -db $(DB) -addr 0.0.0.0:$(PORT)

daemon: ## Run katalogd from source (scans ~/Library/Music, serves 127.0.0.1:7300)
	cargo run -p katalogd -- $(ARGS)

test: ## Go vet and tests, cargo tests
	go vet ./... && go test ./...
	cargo test --workspace

build: ## ./katalog (Go) and target/release/katalogd + katalog (Rust)
	$(MAKE) katalog
	cargo build --release --workspace

katalog: ## The Go web catalogue binary for this machine
	CGO_ENABLED=0 go build -trimpath -ldflags "$(LDFLAGS)" -o katalog .

dist: ## Go catalogue binaries for Linux, macOS and Windows into dist/
	@mkdir -p dist
	@for p in $(PLATFORMS); do \
	  os=$${p%/*}; arch=$${p#*/}; ext=; [ $$os = windows ] && ext=.exe; \
	  echo "building dist/katalog-$$os-$$arch$$ext"; \
	  CGO_ENABLED=0 GOOS=$$os GOARCH=$$arch go build -trimpath -ldflags "$(LDFLAGS)" -o dist/katalog-$$os-$$arch$$ext . || exit 1; \
	done

run: katalog ## Build and run ./katalog serve on PORT
	./katalog serve -db $(DB)

lint: ## gofmt, go vet, cargo fmt, clippy
	gofmt -l -w $$(git ls-files '*.go')
	go vet ./...
	cargo fmt --all
	cargo clippy --workspace

# The Go catalogue is one SQLite file. It is created, with the sample titles
# from seed.json, the first time `katalog serve` opens it.
db-reset: ## Delete the local katalog.db; the next serve recreates it with sample titles
	rm -f $(DB) $(DB)-wal $(DB)-shm
	@echo "removed $(DB); 'make dev' or 'make run' recreates it with the sample titles"

db-seed: ## Import seed.json into katalog.db (titles with the same id are updated)
	$(MAKE) katalog
	./katalog import -db $(DB) seed.json

psql: ## Shell on the local database
	@echo "make $@: not used in this project (SQLite; try: sqlite3 $(DB))"

deploy: ## Build for linux/amd64 and ship to DROPLET
	@echo "make $@: not used in this project (katalogd is a user service; the catalogue publishes with 'katalog export')"

release: ## Tag VERSION, move Unreleased in CHANGELOG.md, push the tag
	@test "$$(git branch --show-current)" = main || { echo "release from main only"; exit 1; }
	@git diff --quiet || { echo "commit first"; exit 1; }
	@grep -q "## \[Unreleased\]" CHANGELOG.md || { echo "CHANGELOG.md needs an Unreleased section"; exit 1; }
	sed -i.bak "s/## \[Unreleased\]/## [Unreleased]\n\n## [$(VERSION)] - $$(date +%Y-%m-%d)/" CHANGELOG.md && rm CHANGELOG.md.bak
	git commit -am "Release $(VERSION)"
	git tag -a v$(VERSION) -m "v$(VERSION)"
	git push && git push --tags

configure-repo: ## One-time GitHub settings: auto-merge, Actions may open PRs, main protected, hold label
	@repo=$$(gh repo view --json nameWithOwner --jq .nameWithOwner); \
	gh api -X PATCH repos/$$repo -F allow_auto_merge=true -F delete_branch_on_merge=true -F allow_squash_merge=true >/dev/null; \
	gh api -X PUT repos/$$repo/actions/permissions/workflow -F default_workflow_permissions=write -F can_approve_pull_request_reviews=true >/dev/null; \
	gh api -X PUT repos/$$repo/branches/main/protection --input - >/dev/null <<< '{"required_status_checks":{"strict":false,"contexts":["ci"]},"enforce_admins":false,"required_pull_request_reviews":null,"restrictions":null,"allow_force_pushes":false,"allow_deletions":false}'; \
	gh label create hold --color B60205 --description "Do not auto-merge" --force >/dev/null; \
	echo "$$repo configured"

clean: ## Remove build output, the exported site and the local database
	rm -rf katalog dist site .dev-server $(DB) $(DB)-wal $(DB)-shm
	cargo clean

.PHONY: help dev daemon test build katalog dist run lint db-reset db-seed psql deploy release configure-repo clean
