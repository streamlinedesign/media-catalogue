# House rules for every Streamline project

These rules are the same in every `streamlinedesign` repo. Follow them before
anything project-specific below the line.

## Workflow

- Start from `main`. Work on a branch named `claude/<short-topic>` (the
  default for Claude threads) or `feat/<topic>` for a person.
- Before pushing: `make test` and `make build` must pass. No exceptions.
- Every push to a `claude/*` or `feat/*` branch opens a pull request by
  itself (`.github/workflows/auto-pr.yml`). CI runs `make test` and
  `make build`. When CI is green the PR merges and the branch is deleted.
  Add the label `hold` to a PR to stop it merging on its own.
- Never push straight to `main`. Never force-push a shared branch.
- One PR is one thing. Keep the description to what changed and why.
- A PR that changes behaviour bumps `VERSION` and adds a line to
  `CHANGELOG.md` under `Unreleased`. `make release` turns Unreleased into a
  dated, tagged version.

## The Makefile is the only entry point

Every repo has the same targets. Never tell a person to run a raw
`go run`, `npm`, `cmake` or `docker` command when a target exists; add the
target instead.

| Target | Does |
|---|---|
| `make dev` | Runs the project locally on its port(s) from `PORTS.md`, reloading on change. |
| `make test` | All tests. Must pass before any push. |
| `make build` | Production build into `./dist` or one binary at `./<name>`. |
| `make lint` | Formatters and linters; fixes what it can. |
| `make db-reset` | Drops and recreates the local database with migrations. |
| `make db-seed` | Loads demo data from `seed/` so the app is usable at once. |
| `make deploy` | Builds for Linux and ships to the droplet named in `.env`. |
| `make release` | Tags `VERSION`, moves Unreleased in the changelog, pushes the tag. |
| `make clean` | Removes build output and local databases. |

A target that doesn't apply (no database, nothing to deploy) stays in the
Makefile and prints `not used in this project`, so `make <anything>` never
fails with "no rule".

## Ports

Each project owns a block of ten ports, listed in `PORTS.md` in the
template repo. Nothing runs on 8080, 3000, 5173 or any tool's default.
`make dev` reads `PORT` (and `WEB_PORT` where there is a separate dev
server) from `.env`; `.env.example` holds the project's numbers. In dev a
server binds all interfaces (`0.0.0.0:$PORT`) so a phone on the same Wi-Fi
can open it at `http://<computer-ip>:$PORT`; `make dev` prints that URL. In
production it listens on `127.0.0.1:$PORT` behind Caddy.

## Secrets and config

- `.env` is local and git-ignored. `.env.example` is committed and lists
  every variable with a safe placeholder. Keep the two in step.
- On a server the same variables live in `/etc/<name>/env`, read by the
  systemd unit's `EnvironmentFile`. Never in the unit file, never in Caddy.
- API tokens come from the owner's password manager, nowhere else. Never
  commit one, never paste one into a PR, log or chat. If one leaks, say so
  immediately so it can be rotated.
- CI secrets go in the repo's Actions secrets.

## Databases and demo data

Where a project has a database (Postgres by default, SQLite for small
single-binary tools):

- Migrations live in `migrations/` and run on start and on `make db-reset`.
- `seed/` holds demo data that makes the app presentable in one command:
  a few realistic rows per table, plus one account `demo` / `demo`.
  `make db-seed` loads it; `make db-reset` gives a clean slate. Keep the
  seed updated when the schema changes, or `make test` should fail.

## Todo

`TODO.md` at the repo root is the project's working list: a short `Now`
section, a `Next` section, and `Later`. Move an item to `Done` with the
version it shipped in. When starting work, read it; when finishing, update
it. The project dashboard reads this file.

## Style

One Streamline design system, kept in `streamlinedesign/design-system` and
published as a Design System artifact. Two colours, two modes, two faces:

- **Paper** `#fffabd` and **ink** `#050505`, the ink ramp between them, and
  one signal colour for live/danger. Nothing else. No navy, no greys that
  aren't on the ramp.
- **Light = Swiss**: ink on paper, solid fills, Schibsted Grotesk for
  headlines (800, tight), IBM Plex Mono for everything small.
- **Dark = Blueprint**: paper on ink, outlines instead of fills, IBM Plex
  Mono alone, cards invert on hover. Games and instruments default to it.
- **Press** register (DDT and anything editorial): a serif for headlines
  and body, mono for metadata; still the two colours.
- No eyebrow text, ever: no small uppercase label sitting above a
  heading. Metadata goes below or beside, in mono, sentence case.
- No Geist, no Inter, no third typeface. Projects don't pick fonts; the
  system does.
- Pin the kit's CSS to a tagged version and use its tokens; if it lacks
  something, add it to the design system in its own PR.
- Pages must work at 360px wide, in both modes, and with keyboard only.
- Copy is sentence case, short, no exclamation marks.

## Deploy

Production is a Debian droplet: one Caddy in front, each project a
systemd service as its own user, listening on `127.0.0.1:$PORT`, data in
`/var/lib/<name>`, config in `/etc/<name>/env`. `deploy/` in the repo holds
the unit file and the Caddy site block; `make deploy` builds for
`linux/amd64`, copies the binary and restarts the service. A project that
is a static site deploys to GitHub Pages instead.

## Writing

READMEs and docs are plain, concrete and short. Say what a thing is, how to
run it, and what is still open. No marketing language.

---

## This project

**media-catalogue** is Katalog, a personal media library: the Rust workspace
in `crates/` (`katalog-core`, the `katalogd` daemon with symphonia/ALSA
playback and LRCLIB lyrics, the `katalog` CLI, the iced `katalog-window`).

- Ports: `4180` (`PORT`) is where `make dev` runs katalogd; `4181` reserved.
  The installed katalogd keeps its own `--bind` / `KATALOG_BIND`
  (`127.0.0.1:7300`) and runs as a user service (`make install`).
- `make dev` and `make daemon` run katalogd from source. `make build` needs
  Rust 1.85+ and `libasound2-dev pkg-config`, or `nix develop` from
  `flake.nix`.
- `db-reset` and `db-seed` are not used: katalogd's SQLite file follows the
  library on disk.
- `release.yml` builds the three Linux binaries on a `v*` tag (`make dist`).
- The earlier Go web catalogue was removed in 0.16.1.
