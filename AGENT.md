# Wish — Agent Guidance

> **Last Updated**: 2026-07-20 — v0.6.0 (ansible deploy + full pipeline verification)

---

# Section 1: Agent Reference

This section is **static** — it's the system prompt for any agent working on this project.

---

## Project Context

**Wish** is a song request server. Guests search across Spotify, YouTube, and SoundCloud and submit track links. The server downloads tracks through a multi-stage pipeline (deemix, then spotDL, then yt-dlp fallback). Downloaded files are served over HTTPS for the companion tool **Deck Feeder** (`github.com/momokli/deck-feeder`).

**Stack**: Rust (Axum/SQLx/SQLite), embedded SPA frontend (vanilla JS/HTML/CSS).
**Deployment target**: Hetzner VPS (projectmellon.de), behind Caddy reverse proxy.

A working Python/FastAPI prototype already runs at `wish.zukkafabrik.de`. This
repo is the Rust rewrite.

---

## Key Principles

### Workflow

1. **`main` is always clean** — never commit directly. Every change goes through
   a feature branch: `feat/short-description` or `fix/short-description`.
2. **Plan first** — every task starts with a Plan entry in Section 2. User reviews
   the plan, then agents are spawned.
3. **Additive migrations** — never modify existing migration files. New schema
   changes get a new migration file.
4. **Testing** — every endpoint tested via integration tests (fresh SQLite, seeded
   data, hit API, assert exact results). `cargo test` is the single source of truth.
5. **Portfolio quality** — public repo, clean code, good README, AGENT.md. This
   represents you professionally.

### Architecture

- **Axum** for HTTP, **SQLx** for SQLite, **rspotify** for Spotify search
- **Embedded frontend** via `rust-embed` — no separate dev server
- **Config priority** (highest wins): env vars > `~/.config/wish/config.toml` > defaults
- **Two-stage download**: deemix-pyweb (HTTP API) → spotDL (CLI fallback)
- **No OAuth for Spotify search** — client credentials flow only
- **Deemix runs as a Docker container** on the same host, exposed on `http://localhost:6596`

---

## Config (`config.toml`)

Secrets live in `~/.config/wish/config.toml` on the server:

```toml
[spotify]
client_id     = "your_spotify_client_id"
client_secret = "your_spotify_client_secret"

[deemix]
base_url = "http://localhost:6596"       # deemix-pyweb API

[download]
output_dir = "/opt/download-service/downloads/tracks"
max_per_user = 5                          # rate limit per browser session
```

**Override with env vars**: `WISH_SPOTIFY_CLIENT_ID=...`, `WISH_DOWNLOAD_OUTPUT_DIR=...`, etc.

Dev-only env vars:

- `DATABASE_URL` — default `sqlite:wish.db`
- `WISH_PORT` — default `3000`

---

## Agent Workflow: Before You Code

### Quick Orientation

```bash
# 1. Establish build baseline
cargo build 2>&1 | tail -5

# 2. Get the CURRENT database schema
rm -f /tmp/wish_test.db
DATABASE_URL=sqlite:/tmp/wish_test.db cargo run -- serve &
sleep 2
sqlite3 /tmp/wish_test.db ".schema"
kill %1 2>/dev/null; rm -f /tmp/wish_test.db

# 3. List actual source modules
ls src/*.rs | sort

# 4. Check current git branch + dirty state
git branch --show-current && git status --short | head -20
```

### Schema Rules

- **Never reconstruct the schema from migration files.** Query the live DB.
- `sqlite3 wish.db ".schema"` IS the canonical schema. Trust it over plan snippets.

### Testing

- **Every API endpoint must have an integration test.** Fresh in-memory SQLite,
  run all migrations, seed hand-crafted data, hit the endpoint, assert exact results.
- Unit tests go in `#[cfg(test)] mod tests` within the source file.
- Integration tests go in `tests/api_*.rs`.
- Run: `cargo test` — must pass with zero failures.

---

## API Endpoints (target)

These are what the Python prototype currently serves. The Rust rewrite must
implement all of them plus the new `/tracks` endpoint for Deck Feeder.

### Public (guest-facing)

| Endpoint                    | Method | Description                                                              |
| --------------------------- | ------ | ------------------------------------------------------------------------ |
| `/`                         | GET    | Embedded SPA frontend (search + request UI)                              |
| `/search?q={query}&limit=5` | GET    | Spotify track search                                                     |
| `/download`                 | POST   | Submit `{"url": "spotify:track:..."}` for download                       |
| `/queue`                    | GET    | List submitted tracks with download status                               |
| `/stats`                    | GET    | `{total, ready, failed, pending}`                                        |
| `/health`                   | GET    | `{status:"ok", deemix_configured, spotify_configured, spotdl_available}` |

### Deck Feeder integration (NEW)

| Endpoint                | Method | Description                                               |
| ----------------------- | ------ | --------------------------------------------------------- |
| `/tracks`               | GET    | List downloadable files: `[{filename, size, url, ready}]` |
| `/downloads/{filename}` | GET    | Serve a downloaded file (static file server)              |

### Admin (future)

| Endpoint            | Method | Description             |
| ------------------- | ------ | ----------------------- |
| `/admin/retry/{id}` | POST   | Retry a failed download |
| `/admin/reset`      | POST   | Clear all submissions   |

---

## Download Pipeline

Two-stage, non-blocking:

```
POST /download {url: "spotify:track:xxx"}
  → INSERT INTO submissions (url, status="pending", created_at)
  → Background worker picks it up:
    1. deemix: POST http://localhost:6596/api/addToQueue {url}
       → Polls GET /api/getQueue until status != "queued"/"downloading"
       → Success → status="ready", file written to output_dir
    2. spotDL (fallback): if deemix fails or returns "not on deezer"
       → spotdl download <spotify_url> --output <output_dir>
       → Success → status="ready"
       → Failure → status="failed", error logged
  → UPDATE submissions SET status, filename, error
```

**Status lifecycle**: `pending` → `stage2_deemix` → `stage3_spotdl` → `ready` | `failed`

---

## Database Schema (v1)

```sql
CREATE TABLE submissions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    spotify_url TEXT NOT NULL,
    track_title TEXT,
    track_artist TEXT,
    cover_url TEXT,
    source TEXT NOT NULL DEFAULT 'spotify',  -- spotify, youtube, soundcloud
    status TEXT NOT NULL DEFAULT 'pending',  -- pending, stage2_deemix, stage3_spotdl, ready, failed
    filename TEXT,
    file_size INTEGER,
    error_message TEXT,
    created_at INTEGER DEFAULT (unixepoch()),
    updated_at INTEGER DEFAULT (unixepoch())
);

CREATE INDEX idx_submissions_status ON submissions(status);
CREATE INDEX idx_submissions_created ON submissions(created_at);
```

---

## Current State

| What                    | Status                                                 |
| ----------------------- | ------------------------------------------------------ |
| Python prototype        | ✅ Running at wish.zukkafabrik.de (uvicorn/FastAPI)    |
| Frontend (HTML/CSS/JS)  | ✅ Working — search + request UI with two tabs         |
| Spotify search          | ✅ via rspotify client credentials                     |
| Deemix download         | ✅ HTTP calls to deemix-pyweb Docker container         |
| spotDL fallback         | ✅ CLI call, installed on Hetzner                      |
| Rust rewrite            | ⬜ This repo — starting now                            |
| Deck Feeder integration | ⬜ Needs `/tracks` + `/downloads/{filename}` endpoints |

### What to extract from the Python prototype

- The frontend HTML/CSS/JS (the `/` response) — port into `frontend/index.html`
- The Spotify search logic (client credentials, query construction)
- The deemix queue interaction pattern (polling loop, status mapping)
- The spotDL CLI invocation pattern
- The config file structure (secrets locations, env var names)

### What to improve in the Rust rewrite

- Proper SQLite instead of JSON file state
- Background download worker (tokio task) instead of blocking FastAPI calls
- Actual file listing API for Deck Feeder
- Static file serving for downloaded tracks
- Proper error handling (the Python prototype silently swallows many errors)
- Tests

---

## Project Structure (target)

```
wish/
├── Cargo.toml
├── AGENT.md                          ← you are here
├── CHANGELOG.md
├── README.md
├── src/
│   ├── main.rs                       # CLI (clap), config loading, server start
│   ├── config.rs                     # Config struct, TOML + env loading
│   ├── api.rs                        # All Axum route handlers
│   ├── db.rs                         # SQLite queries, schema, migrations
│   ├── spotify.rs                    # Spotify search client (rspotify)
│   ├── deemix.rs                     # Deemix HTTP client (reqwest)
│   ├── downloader.rs                 # Background download worker
│   └── models.rs                     # Shared types (Submission, ApiResponse, etc.)
├── frontend/
│   └── index.html                    # Embedded SPA (search + request UI)
├── migrations/
│   └── 001_initial_schema.sql        # CREATE TABLE submissions + indexes
└── tests/
    ├── common/
    │   └── mod.rs                    # Test helpers (in-memory DB, seed data)
    └── api_submissions.rs            # Integration tests for all endpoints
```

---

## Dependencies (rationale)

| Crate                                  | Why                                                     |
| -------------------------------------- | ------------------------------------------------------- |
| `axum` 0.8                             | HTTP framework (same as momos-music-manager)            |
| `sqlx` 0.8                             | SQLite with async, compile-time query checking          |
| `rspotify` 0.15                        | Spotify search via client credentials (no OAuth needed) |
| `rust-embed` 8                         | Embed frontend HTML/CSS/JS in binary                    |
| `reqwest` 0.12                         | HTTP client for deemix-pyweb API calls                  |
| `clap` 4                               | CLI (`wish serve`)                                      |
| `tower-http` 0.5                       | CORS middleware                                         |
| `toml` 0.8 + `dirs` 6 + `dotenvy` 0.15 | Config loading                                          |
| `uuid` 1                               | Generate unique IDs for submissions                     |
| `chrono` 0.4                           | Timestamps                                              |
| `tempfile` 3                           | Test file creation                                      |

**NOT included** (unlike momos-music-manager):

- No `lofty` (audio metadata) — files come pre-tagged from deemix/spotDL
- No `candle` (ML embeddings) — no tag curation
- No `tower-http/ws` (WebSocket) — no real-time sync
- No `soundcloud-rs` / `youtube` — only Spotify search for now

---

## Dev Commands

```bash
# Build
cargo build

# Run (default port 3000)
cargo run -- serve

# Run on custom port
cargo run -- serve --port 8080

# Run tests
cargo test

# Run a specific test file
cargo test --test api_submissions

# Create a new migration
touch migrations/002_description.sql

# Check DB schema
sqlite3 wish.db ".schema"

# Test with curl
curl localhost:3000/health
curl "localhost:3000/search?q=daft+punk&limit=3"
curl -X POST localhost:3000/download -H 'Content-Type: application/json' -d '{"url":"spotify:track:4cOdK2wGLETKBW3PvgPWqT"}'
curl localhost:3000/queue
curl localhost:3000/stats
```

---

---

# Section 2: Active Plans

This section is **dynamic** — plans are appended, updated, and checked off as work progresses.

**Lifecycle**: `proposed` → `approved` → `in-progress` → `done`

---

## Plan: rust-rewrite-v1

**Status**: done
**Branch**: `feat/rust-rewrite-v1`
**Ready for review**: no
**Depends on**: nothing (greenfield)
**Migration needed**: yes — `001_initial_schema.sql`

### Description

Full Rust rewrite of the existing Python/FastAPI wish server. Port the frontend,
implement all existing endpoints, add Deck Feeder integration endpoints, implement
the two-stage download pipeline with background worker.

### Phases

#### Phase 1: Project skeleton + config + DB

1. `Cargo.toml` — done ✅
2. `src/main.rs` — CLI with `serve` subcommand, config loading, embedded frontend
3. `src/config.rs` — `Config` struct, TOML + env loading, priority: env > TOML > defaults
4. `src/db.rs` — `Submission` type, `run_migrations()`, basic CRUD queries
5. `migrations/001_initial_schema.sql` — `submissions` table + indexes
6. `src/models.rs` — shared types (`ApiResponse<T>`, `SubmissionResponse`, etc.)

#### Phase 2: API endpoints (read-only first)

7. `src/api.rs` — router + handlers:
   - `GET /` — serve embedded `frontend/index.html`
   - `GET /health` — `{status, deemix_configured, spotify_configured, spotdl_available}`
   - `GET /stats` — `{total, ready, failed, pending}` from DB
   - `GET /queue` — list submissions with status

#### Phase 3: Spotify search + submission

8. `src/spotify.rs` — `SpotifyClient` using rspotify client credentials
   - `search_tracks(query, limit) -> Vec<SearchResult>`
9. `GET /search?q=...&limit=5` — Spotify search endpoint
10. `POST /download` — accept `{url}`, validate (Spotify URL), INSERT into DB

#### Phase 4: Download pipeline

11. `src/deemix.rs` — `DeemixClient` wrapping reqwest calls to deemix-pyweb
    - `add_to_queue(url)` — POST /api/addToQueue
    - `get_queue_status()` — GET /api/getQueue, find our item
    - `poll_until_done(url, timeout)` — loop until ready/failed
12. `src/downloader.rs` — `DownloadWorker` background task
    - Picks up pending submissions from DB
    - Stage 1: deemix → poll until done
    - Stage 2: spotDL fallback (if deemix failed)
    - Updates submission status in DB
    - Handles output file detection (find downloaded file by title/artist)

#### Phase 5: Deck Feeder integration

13. `GET /tracks` — list downloadable files: `[{filename, size, url, ready}]`
    - Scans `output_dir`, matches against `submissions` table
    - Returns JSON for Deck Feeder to consume
14. `GET /downloads/{filename}` — serve static file from `output_dir`
    - Range request support for audio streaming
    - Security: only serve files that are in the `submissions` table

#### Phase 6: Frontend port

15. `frontend/index.html` — port the existing Python prototype's frontend:
    - Two tabs: Search + Requests
    - Spotify search with debounced typeahead
    - "Want" button → POST /download
    - Queue tab with status badges (ready/pending/failed)
    - Client-side rate limiting (max_per_user via localStorage)
    - Event name from localStorage
    - Source badges (Spotify, YouTube, SoundCloud)
    - Toast notifications

#### Phase 7: Tests + polish

16. `tests/common/mod.rs` — test helpers (in-memory DB, seed data)
17. `tests/api_submissions.rs` — integration tests for all endpoints
18. `README.md` — project README with setup instructions, screenshots
19. `CHANGELOG.md` — initial release notes

### Files to create

| File                                | Phase |
| ----------------------------------- | ----- |
| `migrations/001_initial_schema.sql` | 1     |
| `src/main.rs`                       | 1     |
| `src/config.rs`                     | 1     |
| `src/db.rs`                         | 1     |
| `src/models.rs`                     | 1     |
| `src/api.rs`                        | 2     |
| `src/spotify.rs`                    | 3     |
| `src/deemix.rs`                     | 4     |
| `src/downloader.rs`                 | 4     |
| `frontend/index.html`               | 6     |
| `tests/common/mod.rs`               | 7     |
| `tests/api_submissions.rs`          | 7     |
| `README.md`                         | 7     |
| `CHANGELOG.md`                      | 7     |

### Acceptance Criteria

- [x] `cargo build` passes
- [x] `cargo run -- serve` starts the server
- [x] `curl localhost:3000/health` returns `{"status":"ok"}`
- [x] `curl localhost:3000/stats` returns `{total:0,ready:0,failed:0,pending:0}`
- [x] `curl -X POST localhost:3000/download -H 'Content-Type: application/json' -d '{"url":"spotify:track:xxx"}'` creates a submission
- [x] `curl localhost:3000/queue` returns the submission with `status:pending`
- [x] `curl "localhost:3000/search?q=daft+punk"` returns Spotify search results (requires Spotify credentials)
- [x] Background worker processes pending submissions (deemix → spotDL)
- [x] `curl localhost:3000/tracks` returns list of downloaded files
- [x] `curl localhost:3000/downloads/somefile.mp3` serves the file
- [x] Embedded frontend loads at `/` with search + request UI
- [x] `cargo test` passes (all integration tests)
- [ ] Frontend matches the existing Python prototype behavior
- [ ] Binary can be deployed to Hetzner, replacing the Python service

### Agent Decomposition (TDD, 6 agents, zero file conflicts)

| Agent | Files                                                                                 | Phase | Work                                            |
| ----- | ------------------------------------------------------------------------------------- | ----- | ----------------------------------------------- |
| **A** | `migrations/001_*.sql`, `src/main.rs`, `src/config.rs`, `src/models.rs`, `Cargo.toml` | 1     | Project skeleton, CLI, config, DB types         |
| **B** | `src/db.rs`, `src/api.rs` (read endpoints)                                            | 1-2   | SQLite layer + /health, /stats, /queue handlers |
| **C** | `src/spotify.rs`, `src/api.rs` (search+download)                                      | 3     | Spotify client + /search + /download endpoints  |
| **D** | `src/deemix.rs`, `src/downloader.rs`                                                  | 4     | Deemix client + background download worker      |
| **E** | `src/api.rs` (tracks+downloads), `frontend/index.html`                                | 5-6   | Deck Feeder endpoints + frontend port           |
| **F** | `tests/common/mod.rs`, `tests/api_submissions.rs`, `README.md`, `CHANGELOG.md`        | 7     | Tests + documentation                           |

**Write scope verification — zero overlap:**

- Agents A-F all touch different files
- Agent E touches `src/api.rs` but only for the tracks/downloads endpoints (distinct functions)
- Agent F touches only test/doc files

All 6 agents can run in parallel.

### Per-Agent Task Briefs

#### Agent A: Project skeleton + config

Create the Rust project skeleton:

1. Read the existing `Cargo.toml` and `src/main.rs` (already generated by `cargo init`)
2. Write `src/config.rs`:
   - `Config` struct with `Spotify { client_id, client_secret }`, `Deemix { base_url }`, `Download { output_dir, max_per_user }`
   - `Config::load()` — env vars > `~/.config/wish/config.toml` > defaults
   - Use `dirs`, `toml`, `dotenvy`
   - Env var naming: `WISH_SPOTIFY_CLIENT_ID`, `WISH_DOWNLOAD_OUTPUT_DIR`, etc.
3. Write `src/models.rs`:
   - `ApiResponse<T> { data: T }` — standard JSON wrapper
   - `Submission` struct (matches DB schema)
   - `SearchResult { title, artist, cover_url, spotify_url, source, duration_ms }`
   - `StatsResponse { total, ready, failed, pending }`
   - `QueueResponse { tasks: Vec<SubmissionResponse> }`
4. Update `src/main.rs`:
   - CLI with clap: `wish serve [--port PORT]`
   - Load config, create SQLite pool, run migrations, build router, start server
   - Register `rust-embed` for `frontend/`
5. Create `migrations/001_initial_schema.sql` with the `submissions` table + indexes (see schema above)
6. Verify: `cargo build` compiles cleanly

#### Agent B: SQLite layer + read endpoints

Implement the database layer and read-only API endpoints:

1. Write `src/db.rs`:
   - `run_migrations(pool)` — read .sql files from `migrations/`, run in order
   - `get_submissions(pool, status_filter) -> Vec<Submission>`
   - `get_stats(pool) -> (total, ready, failed, pending)`
   - `insert_submission(pool, url, title, artist, cover_url, source) -> Submission`
   - `update_submission_status(pool, id, status, filename, file_size, error)`
2. Write `src/api.rs` (create router function):
   - `GET /health` — check spotify client_id set, deemix base_url set, spotdl on PATH
   - `GET /stats` — query DB, return counts
   - `GET /queue` — query DB for all submissions, return as JSON
   - `GET /` — serve embedded `frontend/index.html` (can be a stub for now)
3. Wire into `src/main.rs` (Agent A provides the router builder)
4. Verify: `cargo build` compiles; `cargo run -- serve` starts and responds to curl

#### Agent C: Spotify search + submission endpoint

Implement Spotify integration and the download submission endpoint:

1. Write `src/spotify.rs`:
   - `SpotifyClient::new(client_id, client_secret)` — rspotify with client credentials
   - `search_tracks(query, limit) -> Vec<SearchResult>` — call rspotify search, map to our types
   - Handle errors gracefully (network, auth) — return empty vec or error
2. Extend `src/api.rs`:
   - `GET /search?q=...&limit=5` — validate query (min 2 chars), call spotify client, return JSON
   - `POST /download` — parse `{url}`, validate Spotify URL format, resolve track metadata via rspotify, INSERT into DB, trigger background worker notification
3. For the download handler: use a `tokio::sync::mpsc` channel to notify the background worker (Agent D will implement the receiver side). Or use `tokio::sync::Notify`.
4. Verify: `curl "localhost:3000/search?q=daft+punk"` returns results; `curl -X POST localhost:3000/download ...` creates a submission

#### Agent D: Deemix client + background download worker

Implement the download pipeline:

1. Write `src/deemix.rs`:
   - `DeemixClient::new(base_url)` — wraps reqwest client
   - `add_to_queue(spotify_url) -> Result<()>` — POST /api/addToQueue
   - `get_queue() -> Result<Vec<DeemixQueueItem>>` — GET /api/getQueue
   - `find_by_url(url) -> Option<DeemixQueueItem>` — find our item in queue
   - `DeemixQueueItem { url, status, track_count_total, track_count_downloaded, errors }`
2. Write `src/downloader.rs`:
   - `DownloadWorker::new(pool, deemix_client, output_dir, notify_rx)`
   - Background loop: wait for notification → query pending submissions → process each
   - Processing: update status to `stage2_deemix` → call deemix → poll until done → if success, find output file, update status to `ready` → if fail, try spotDL
   - spotDL fallback: `spotdl download <url> --output <output_dir>`
   - Update DB after each stage
   - Log every step at `info!` level
3. Wire into `src/main.rs`: spawn `DownloadWorker` as a tokio task
4. Verify: submit a track → background worker picks it up → status transitions visible in DB

#### Agent E: Deck Feeder endpoints + frontend port

Implement the Deck Feeder API and port the frontend:

1. Extend `src/api.rs`:
   - `GET /tracks` — scan `output_dir`, match files against `submissions` table, return JSON:
     ```json
     [
       {
         "filename": "Artist - Title.mp3",
         "size": 11234567,
         "url": "/downloads/Artist%20-%20Title.mp3",
         "ready": true
       }
     ]
     ```
   - `GET /downloads/{filename}` — serve file from `output_dir` with correct Content-Type
   - Security: verify the file is in the `submissions` table (prevent path traversal)
   - Support `Range` header for audio streaming
2. Write `frontend/index.html`:
   - Port the existing Python prototype's frontend (see the HTML/CSS/JS from the current server)
   - Same two-tab layout (Search + Requests)
   - Same Spotify search with debounce
   - Same "Want" button → POST /download
   - Same queue display with status badges
   - Same localStorage rate limiting
   - Same toast notifications
   - Adapt API calls to use relative URLs (no hardcoded host)
3. Verify: `cargo build` with embedded frontend; visit `localhost:3000` → full UI loads

#### Agent F: Tests + documentation

Write tests and docs:

1. `tests/common/mod.rs`:
   - `create_test_db() -> Pool<Sqlite>` — in-memory SQLite, run all migrations
   - `test_app() -> (Router, Pool<Sqlite>)` — build test app with test DB
   - `seed_submission(pool, status, ...)` — insert a test submission
2. `tests/api_submissions.rs`:
   - `health_returns_ok` — GET /health → 200 with status field
   - `stats_starts_empty` — GET /stats → all zeros
   - `search_requires_query` — GET /search without q → 400
   - `download_creates_submission` — POST /download → 200, submission appears in /queue
   - `download_invalid_url` — POST /download with garbage URL → 400
   - `queue_returns_submissions` — seed some, GET /queue → correct count
   - `tracks_returns_files` — seed submissions with filenames, GET /tracks → correct list
   - `downloads_serves_file` — create temp file, seed submission, GET /downloads/filename → file content
   - `downloads_404_unknown` — GET /downloads/nonexistent → 404
   - `stats_counts_correct` — seed various statuses, GET /stats → correct counts
3. `README.md`:
   - Project description
   - Setup instructions (clone, config, run)
   - Deployment guide (Hetzner, Caddy, systemd)
   - API documentation (all endpoints with curl examples)
   - Architecture diagram (mermaid)
   - Screenshot of the frontend
4. `CHANGELOG.md`:
   - v0.1.0: Initial Rust rewrite, all endpoints, Deck Feeder integration

### Execution Order

Agents A and B FIRST (foundation — DB + config + basic API). Then C, D, E, F can run in parallel (all depend on A+B but don't conflict with each other).

---

## Plan: multi-source-search

**Status**: done
**Branch**: `feat/rust-rewrite-v1`
**Ready for review**: no
**Depends on**: rust-rewrite-v1

### Description

Add multi-source search (Spotify + YouTube + SoundCloud), yt-dlp download support,
and a redesigned frontend with filter bar, parallel fetching, and placeholder cards.

### Changes

#### Backend

1. **`src/youtube.rs`** (NEW) — YouTube search via `yt-dlp ytsearchN:query --dump-json`
2. **`src/soundcloud.rs`** (NEW) — SoundCloud search via `yt-dlp scsearchN:query --dump-json`
3. **`src/api.rs`** — `/search` now supports `?source=spotify|youtube|soundcloud`
   - Default: `spotify` (backward compatible)
   - `/download` now accepts `{"url": "...", "source": "youtube|soundcloud|spotify"}`
   - Auto-detects source from URL if `source` field omitted
   - `resolve_via_ytdlp()` for metadata resolution of non-Spotify URLs
   - Health endpoint now reports `ytdlp_available`
4. **`src/models.rs`** — `SearchResponse.source`, `DownloadRequest.source`, `HealthResponse.ytdlp_available`
5. **`src/downloader.rs`** — Multi-stage pipeline:
   - `spotify`: deemix → spotDL → yt-dlp (3-stage fallback)
   - `youtube` / `soundcloud`: yt-dlp directly (bestaudio + extract audio)
   - `DownloadWorker` takes `ytdlp_available` flag
6. **`src/main.rs`** — Checks yt-dlp on PATH at startup, passes to `AppState` and worker

#### Frontend

7. **`frontend/index.html`** — Complete redesign:
   - **Filter bar**: 3 toggle buttons (Spotify / YouTube / SoundCloud) with colored dots
   - **Parallel fetching**: fires 3 independent `fetch()` calls simultaneously
   - **Placeholder cards**: skeleton shimmer animation while waiting for results
   - **Per-source sections**: results grouped under colored source headers with counts
   - No layout jumping — placeholders pre-reserve space
   - Auto-detects already-submitted URLs from queue on load

### Acceptance Criteria

- [x] `cargo build` passes (0 warnings)
- [x] `cargo test` passes (20/20)
- [x] `/health` reports `ytdlp_available`
- [x] `/search?q=...&source=youtube` returns YouTube results (yt-dlp required)
- [x] `/search?q=...&source=soundcloud` returns SoundCloud results (yt-dlp required)
- [x] `/search?q=...&source=spotify` returns Spotify results (backward compat)
- [x] `/download` accepts YouTube/SoundCloud URLs with auto source detection
- [x] Download worker handles `youtube`/`soundcloud` sources via yt-dlp
- [x] Frontend filter bar toggles sources on/off
- [x] Frontend shows skeleton placeholders during search
- [x] Frontend fires parallel requests and renders as results arrive

---

## Plan: full-pipeline-verification

**Status**: done
**Branch**: `feat/rust-rewrite-v1`
**Ready for review**: no
**Depends on**: multi-source-search, deemix-auth

### Description

End-to-end verification that ALL providers work, ALL fallbacks function,
the download pipeline is correct, dufs file serving is correct, and the
frontend correctly displays download sources. TDD: test first, fix second.

### Verification Checklist

#### Infrastructure

- [x] `files.wish.zukkafabrik.de` serves from the SAME directory as wish/deemix
- [x] No file copying between dirs — single `/opt/download-service/downloads`
- [x] Deemix download dir is stable (duplicate downloads auto-skipped)

#### Download Pipeline — Spotify

- [x] Spotify track → L1 deemix downloads at 320kbps (or FLAC fallback)
- [x] If deemix fails → L2 spotDL attempts download
- [x] If spotDL fails → L3 yt-dlp searches and downloads
- [x] DB shows `"downloaded via deemix"` (or spotDL / yt-dlp)
- [x] Frontend shows green via-badge with correct source
- [x] File appears on disk in `/opt/download-service/downloads`
- [x] File visible at `https://files.wish.zukkafabrik.de/<filename>`

#### Download Pipeline — YouTube

- [x] YouTube URL → yt-dlp downloads directly
- [x] Uses `ytsearch1:Artist - Title` to avoid bot detection
- [x] DB shows `"downloaded via yt-dlp"`
- [x] Frontend shows green via-badge

#### Download Pipeline — SoundCloud

- [x] SoundCloud URL → yt-dlp downloads directly
- [x] DB shows `"downloaded via yt-dlp"`

#### Search

- [x] Spotify search returns results
- [x] YouTube search returns results (yt-dlp ytsearchN)
- [x] SoundCloud search returns results (yt-dlp scsearchN)
- [x] Filter bar toggles show/hide without re-searching
- [x] Cache avoids re-fetching same query

#### Frontend

- [x] Requests tab shows all submissions with status
- [x] Ready tracks show "downloaded via <source>" in green
- [x] Failed tracks show short error reason in red
- [x] Stat boxes show Total / Ready / Pending / Failed
- [x] Track titles display human-readable fallbacks for null metadata

#### Deemix Configuration

- [x] Spotify URLs submitted directly to deemix (`add_to_queue`)
- [x] 320kbps MP3 configured (`maxBitrate: "3"`)
- [x] Bitrate fallback enabled
- [x] Spotify plugin enabled + client credentials set
- [x] ARL authenticated

### Agent Decomposition

| Agent | File(s)      | Work                                                                                |
| ----- | ------------ | ----------------------------------------------------------------------------------- |
| **A** | (infra)      | ✅ Verify dufs serves from same dir, test file URLs work, verify no copying         |
| **B** | (spotify)    | ✅ Reset DB, submit Spotify track, verify L1→L2→L3 fallback, check "downloaded via" |
| **C** | (youtube)    | ✅ Reset DB, submit YouTube track, verify yt-dlp direct download with ytsearch1:    |
| **D** | (soundcloud) | ✅ Reset DB, submit SoundCloud track, verify yt-dlp download                        |
| **E** | (frontend)   | ✅ Verify search results, filter toggles, requests tab rendering, via-badges        |
| **F** | (deemix)     | ✅ Verify Spotify URL → deemix directly, config correctness, ARL auth               |

**Execution order**: A first (infra), then B/C/D in parallel (downloads), E+F can run anytime.

### Acceptance Criteria

- [x] `files.wish.zukkafabrik.de` shows downloaded files
- [x] Spotify download succeeds through at least one layer, shows "downloaded via <layer>"
- [x] YouTube download succeeds, shows "downloaded via yt-dlp"
- [x] SoundCloud download succeeds (or fails gracefully with clear reason)
- [x] Frontend shows all results correctly
- [x] `curl https://wish.zukkafabrik.de/health` shows all services available
- [x] No file copying — all services write to same directory

## Plan: admin-view

**Status**: in-progress
**Branch**: `feat/rust-rewrite-v1`
**Depends on**: full-pipeline-verification

### Description

Add an `/admin` page with a technical table of all submissions — IDs, URLs,
status, bitrate, container, file size, download source, per-track attempt logs.
Built in two phases: MVP → POC.

### MVP: Basic admin table

#### Backend

1. **Migration `002_admin_fields.sql`** — add columns:
   - `bitrate` TEXT (e.g. "320kbps", "lossless")
   - `container` TEXT (e.g. "mp3", "flac", "m4a")
   - `attempts_json` TEXT (JSON array of attempt logs)

2. **`src/api.rs`** — `/admin` endpoint serves embedded admin HTML
   - `/admin/data` — JSON endpoint returning all submissions with full details

3. **`src/downloader.rs`** — after each download attempt, append to `attempts_json`
   - On success: record `{"layer": "yt-dlp", "file": "...", "bitrate": "...", "container": "mp3"}`
   - Detect container from file extension, attempt bitrate from yt-dlp output

#### Frontend

4. **`frontend/admin.html`** — standalone admin page (separate from guest SPA):
   - Dark-themed table: ID, Title, Artist, Source, Status, Bitrate, Container, Size, Via
   - Filter by status (ready/failed/pending)
   - Sort by any column
   - Auto-refresh every 10s
   - Click row → expand to show attempt logs

### POC: Full admin with attempt timeline

5. Store per-attempt details: command output, timing, errors
6. Admin: timeline view showing each download attempt as a row
7. Export as CSV

### Agent Decomposition

| Agent | Files                                                           | Work                                                                                         |
| ----- | --------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| **A** | `migrations/002_admin_fields.sql`, `src/db.rs`, `src/models.rs` | DB migration, new columns, insert/update helpers                                             |
| **B** | `src/downloader.rs`                                             | Record attempt details (bitrate, container, attempt_json) in try_spotdl/run_ytdlp/try_deemix |
| **C** | `src/api.rs`                                                    | `/admin` + `/admin/data` endpoints, serve embedded admin.html                                |
| **D** | `frontend/admin.html`                                           | Admin table UI with filtering, sorting, auto-refresh                                         |

**Execution order**: A first (DB), then B+C+D in parallel.

---

## Completed Plans

### full-pipeline-verification — completed 2026-07-20

End-to-end verification of all providers, fallback pipeline, dufs file serving,
and frontend display. Fixed SoundCloud routing bug and directory permissions.

### multi-source-search — completed 2026-07-19

Multi-source search (Spotify/YouTube/SoundCloud), yt-dlp download integration,
and redesigned frontend with filter bar, parallel fetching, and placeholder cards.

Files created: `src/youtube.rs`, `src/soundcloud.rs`
Files modified: `src/api.rs`, `src/models.rs`, `src/downloader.rs`, `src/main.rs`, `src/lib.rs`, `frontend/index.html`, `tests/api_submissions.rs`

### rust-rewrite-v1 — completed 2026-07-19

Full Rust rewrite of the Python/FastAPI wish server. All phases completed:

- Phase 1-2: Project skeleton, config, DB, read-only endpoints ✅
- Phase 3: Spotify search + submission ✅
- Phase 4: Download pipeline (deemix + spotDL fallback) ✅
- Phase 5: Deck Feeder integration ✅
- Phase 6: Frontend port ✅
- Phase 7: Tests + documentation ✅

Files created: All 14 files from the plan.
Tests: 9 unit + 11 integration = 20 passing.

---

## Handover

1. Document progress in Section 2 above
2. Run `cargo build` — must pass
3. Run `cargo test` — all tests must pass
4. If you added a new endpoint, verify with `curl` first
5. Update `CHANGELOG.md` with your changes
6. Bump "Last Updated" date at the top of this file
