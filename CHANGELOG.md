# Changelog

All notable changes to this project will be documented in this file.

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
