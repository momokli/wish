# Changelog

All notable changes to this project will be documented in this file.

## [0.7.2] — 2026-08-03

### Added

- **Download verification pipeline**: every download is verified before marking `ready`
  - ISRC extraction and comparison (primary, exact match)
  - Title/artist fuzzy matching with substring tolerance (handles YouTube prefixes)
  - Duration verification (±25% tolerance)
  - Artist verification for all non-yt-dlp sources
  - Wrong files are quarantined to `_rejected/` instead of silently accepted
  - Cross-submission ISRC reassignment (file reassigned if ISRC matches another submission)
- `/admin/deemix-check` endpoint — validates deemix reachability and ARL status
- `duration_ms` column on submissions (migration 20260802140000)
- Transparent per-layer logging: L1/L2/L3 with failure reasons
- YouTube/SoundCloud cover thumbnails in search results
- Test scripts: `scripts/test-reset.sh`, `scripts/test-one-track.sh`, `scripts/test-state.sh`
- Ansible: Deno install for spotDL, spotDL Spotify config, deemix permissions fix

### Changed

- Deemix image: `bockiii/deemix-docker:latest` → `ghcr.io/bambanah/deemix:v4.6.0` (fixes static track_id bug)
- Deemix file detection: UUID polling replaced with snapshot-based filesystem watch
- spotDL: 500ms fixed delay → 60s polling loop with file size stability detection
- spotDL: LookupError/SearchError detected and bailed immediately (no wasted retries)
- Admin: `duration_ms` column added to data view
- Admin: null entries filtered from `attempts_json` count

### Fixed

- Deemix permissions: download dir `chmod 777` for container uid 911
- Deemix `maxBitrate: 1` for free Deezer accounts (was 3/320kbps → `CantStream`)
- spotDL LookupError incorrectly treated as success (exits 0 but no file)
- `scan_recent` only returned most-recent file, hiding files from concurrent downloads
- Artist check now runs even when ISRC matches (Fortuna vs Gippeul on Deezer)

## [0.2.0] — 2026-07-19

### Added

- Multi-source search: Spotify, YouTube, and SoundCloud search support
- YouTube search via yt-dlp (`ytsearchN:query --dump-json`)
- SoundCloud search via yt-dlp (`scsearchN:query --dump-json`)
- `/search?source=youtube|soundcloud|spotify` parameter for per-source queries
- `/download` now accepts `source` field and auto-detects source from URL
- yt-dlp download support for YouTube/SoundCloud submissions
- Deemix → spotDL → yt-dlp 3-stage fallback pipeline for Spotify tracks
- `ytdlp_available` field in `/health` response
- Filter bar on frontend (toggle Spotify/YouTube/SoundCloud on/off)
- Parallel source fetching — 3 independent requests fire simultaneously
- Skeleton placeholder cards with shimmer animation while loading
- Per-source result sections with colored headers and counts
- Frontend auto-detects already-submitted URLs on load

### Changed

- Frontend redesigned for multi-source results with zero layout jumping
- Download worker now source-aware (different pipelines per platform)
- `AppState` includes `ytdlp_available` flag

## [0.1.0] — 2026-07-19

### Added

- Initial Rust rewrite of the Python/FastAPI wish server
- Embedded SPA frontend with search and request UI (vanilla JS/HTML/CSS)
- Spotify search via rspotify client credentials flow
- Download submission endpoint with Spotify URL validation
- Two-stage download pipeline: deemix → spotDL fallback
- Background download worker (tokio task)
- Deck Feeder integration: `/tracks` endpoint and `/downloads/{filename}` file serving with Range support
- SQLite database with migrations (`submissions` table)
- Health check endpoint with service availability info
- Stats endpoint with submission counts
- Queue endpoint listing all submissions
- Config loading: env vars > `~/.config/wish/config.toml` > defaults
- CLI with `wish serve [--port PORT]`
- Full integration test suite (11 tests covering all endpoints)
- Unit tests for DB layer, config, and Spotify URL parsing (9 tests)
