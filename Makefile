SHELL := bash
.DEFAULT_GOAL := help

# ---- Project ----------------------------------------------------------------
# media-catalogue: katalogd, the katalog CLI and katalog-window (crates/).
# Ports from PORTS.md in streamlinedesign/template; .env overrides. In
# `make dev`, PORT is the phone page (all interfaces, behind
# KATALOG_WEB_PASSWORD) and WEB_PORT the local API. The installed service
# keeps its own KATALOG_BIND (127.0.0.1:7300) and KATALOG_WEB.
NAME     ?= katalog
PORT     ?= 4180
WEB_PORT ?= 4181
DROPLET  ?=
# -----------------------------------------------------------------------------

-include .env
export

VERSION := $(shell cat VERSION 2>/dev/null || echo 0.0.0)

help: ## List targets
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk -F ':.*## ' '{printf "  %-14s %s\n", $$1, $$2}'

dev: ## katalogd from source: the phone page on 0.0.0.0:PORT (4180), the API on 127.0.0.1:WEB_PORT
	@test -n "$(KATALOG_WEB_PASSWORD)" || { echo "set KATALOG_WEB_PASSWORD in .env for the phone page"; exit 1; }
	@echo "On your phone: http://$$(hostname -I 2>/dev/null | cut -d' ' -f1):$(PORT)"
	cargo run -p katalogd -- --bind 127.0.0.1:$(WEB_PORT) --web 0.0.0.0:$(PORT) --socket '' $(ARGS)

daemon: ## Run katalogd from source (scans ~/Library/Music, serves 127.0.0.1:7300)
	cargo run -p katalogd -- $(ARGS)

install: build ## Put katalogd, katalog and katalog-window in ~/.local/bin and run katalogd as a user service
	@mkdir -p $(HOME)/.local/bin $(HOME)/.config/systemd/user $(HOME)/.config/katalog
	install -m 755 target/release/katalogd target/release/katalog target/release/katalog-window $(HOME)/.local/bin/
	install -m 644 deploy/katalogd.service $(HOME)/.config/systemd/user/katalogd.service
	@test -f $(HOME)/.config/katalog/env || printf '# KATALOG_LIBRARY=$(HOME)/Library\n# KATALOG_DEVICE=auto\n# KATALOG_BIND=127.0.0.1:7300\n# The phone page on the Wi-Fi, off until both are set:\n# KATALOG_WEB=0.0.0.0:4180\n# KATALOG_WEB_PASSWORD=\n' >$(HOME)/.config/katalog/env
	systemctl --user daemon-reload
	systemctl --user enable --now katalogd
	systemctl --user restart katalogd
	@echo "katalogd runs as a user service; settings in ~/.config/katalog/env, log: journalctl --user -u katalogd -f"

uninstall: ## Stop the user service and remove the binaries
	-systemctl --user disable --now katalogd
	rm -f $(HOME)/.config/systemd/user/katalogd.service $(HOME)/.local/bin/katalogd $(HOME)/.local/bin/katalog $(HOME)/.local/bin/katalog-window
	systemctl --user daemon-reload

test: ## cargo tests for the whole workspace
	cargo test --workspace

build: ## target/release/katalogd, katalog and katalog-window
	cargo build --release --workspace

dist: build ## The three binaries for this machine in dist/katalog-VERSION-linux-ARCH.tar.gz
	@mkdir -p dist
	tar -czf dist/katalog-$(VERSION)-linux-$$(uname -m).tar.gz -C target/release katalogd katalog katalog-window
	@ls dist

lint: ## cargo fmt, clippy
	cargo fmt --all
	cargo clippy --workspace

db-reset: ## Not used: katalogd's database follows the library
	@echo "make $@: not used in this project (katalogd keeps ~/.local/share/katalog/katalog.db and rescans on start)"

db-seed: ## Not used: katalogd reads your own library
	@echo "make $@: not used in this project (point KATALOG_LIBRARY at a folder of music and films)"

psql: ## Shell on the local database
	@echo "make $@: not used in this project (SQLite)"

deploy: ## Build for linux/amd64 and ship to DROPLET
	@echo "make $@: not used in this project (katalogd is a user service: make install)"

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

clean: ## Remove build output
	rm -rf dist
	cargo clean

.PHONY: help dev daemon install uninstall test build dist lint db-reset db-seed psql deploy release configure-repo clean
