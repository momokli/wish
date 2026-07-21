use axum::{
    Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Json, Response},
    routing::{delete, get, post},
};
use serde::Deserialize;
use sqlx::sqlite::SqlitePool;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::sync::Notify;

use crate::config::Config;
use crate::db;
use crate::error::AppError;
use crate::models::*;
use crate::playlists;
use crate::spotify::SpotifyClient;

/// Shared application state.
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Config,
    pub spotify: Option<SpotifyClient>,
    pub download_notify: Arc<Notify>,
    pub ytdlp_available: bool,
    pub spotdl_available: bool,
}

/// Embedded frontend assets (built by scripts/build-html.mjs).
#[derive(rust_embed::RustEmbed)]
#[folder = "frontend/dist/"]
struct FrontendAssets;

/// Build the application router.
pub fn build_router(state: Arc<AppState>) -> Router {
    use tower_http::cors::{Any, CorsLayer};

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // Public endpoints
        .route("/", get(serve_frontend))
        .route("/health", get(health))
        .route("/stats", get(stats))
        .route("/queue", get(queue))
        .route("/search", get(search))
        .route("/download", post(download))
        // Deck Feeder integration
        .route("/tracks", get(tracks))
        .route("/downloads/{filename}", get(serve_download))
        // Playlists
        .route("/playlists", get(playlists_list).post(playlists_add))
        .route("/playlists/{id}", delete(playlists_delete))
        .route("/playlists/{id}/sync", post(playlists_sync))
        // Admin
        .route("/admin", get(serve_admin))
        .route("/admin/data", get(admin_data))
        .layer(cors)
        .with_state(state)
}

// ─── Admin ────────────────────────────────────────────────────────

async fn serve_admin() -> impl IntoResponse {
    match FrontendAssets::get("admin.html") {
        Some(file) => Response::builder()
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .body(Body::from(file.data))
            .expect("infallible: response builder"),
        None => (
            StatusCode::NOT_FOUND,
            "Admin page not found. Place admin.html in frontend/",
        )
            .into_response(),
    }
}

async fn admin_data(State(state): State<Arc<AppState>>) -> Result<Json<Vec<AdminRow>>, AppError> {
    let rows = sqlx::query_as::<_, AdminRow>(
        "SELECT id, track_title, track_artist, spotify_url, source, status, filename, file_size, error_message, bitrate, container, attempts_json, created_at, updated_at, first_available_at FROM submissions ORDER BY created_at DESC"
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| AppError::Internal(format!("DB error: {e}")))?;
    Ok(Json(rows))
}

// ─── Playlists ────────────────────────────────────────────────────

async fn playlists_list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<Playlist>>, AppError> {
    let playlists = db::get_playlists(&state.pool)
        .await
        .map_err(|e| AppError::Internal(format!("DB error: {e}")))?;
    Ok(Json(playlists))
}

async fn playlists_add(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AddPlaylistRequest>,
) -> Result<Json<Playlist>, AppError> {
    let source = body
        .source
        .unwrap_or_else(|| playlists::detect_source(&body.url));

    if source == "unknown" {
        return Err(AppError::BadRequest(
            "Unsupported URL. Please enter a Spotify, YouTube, or SoundCloud link.".to_string(),
        ));
    }

    // Insert with no title initially — sync will fill it in
    let mut playlist = db::insert_playlist(&state.pool, &body.url, &source, None)
        .await
        .map_err(|e| AppError::Internal(format!("DB error: {e}")))?;

    // Do initial sync to resolve the title and import tracks
    if let Err(e) = playlists::sync_one(
        &state.pool,
        &playlist,
        &state.download_notify,
        state.spotify.as_ref(),
    )
    .await
    {
        tracing::warn!("[{}] Initial playlist sync failed: {e}", playlist.id);
        // Re-fetch so the caller gets the updated record (error persisted)
        playlist = db::get_playlist_by_id(&state.pool, playlist.id)
            .await
            .map_err(|e| AppError::Internal(format!("DB error: {e}")))?
            .ok_or_else(|| AppError::Internal("Playlist disappeared after insert".into()))?;
    }

    Ok(Json(playlist))
}

async fn playlists_delete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    let deleted = db::delete_playlist(&state.pool, id)
        .await
        .map_err(|e| AppError::Internal(format!("DB error: {e}")))?;
    if deleted {
        Ok(StatusCode::OK)
    } else {
        Err(AppError::NotFound(format!("Playlist {id} not found")))
    }
}

async fn playlists_sync(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Playlist>, AppError> {
    let playlist = db::get_playlist_by_id(&state.pool, id)
        .await
        .map_err(|e| AppError::Internal(format!("DB error: {e}")))?
        .ok_or_else(|| AppError::NotFound(format!("Playlist {id} not found")))?;

    playlists::sync_one(
        &state.pool,
        &playlist,
        &state.download_notify,
        state.spotify.as_ref(),
    )
    .await
    .map_err(|e| AppError::Internal(format!("Sync failed: {e}")))?;

    let updated = db::get_playlist_by_id(&state.pool, id)
        .await
        .map_err(|e| AppError::Internal(format!("DB error: {e}")))?
        .ok_or_else(|| AppError::Internal("Playlist vanished after sync".into()))?;

    Ok(Json(updated))
}

// ─── Frontend ────────────────────────────────────────────────────

async fn serve_frontend() -> impl IntoResponse {
    match FrontendAssets::get("index.html") {
        Some(file) => {
            let mime = mime_guess::from_path("index.html")
                .first_or_octet_stream()
                .to_string();
            Response::builder()
                .header(header::CONTENT_TYPE, mime)
                .body(Body::from(file.data))
                .expect("infallible: response builder")
        }
        None => (
            StatusCode::NOT_FOUND,
            "Frontend not found. Place index.html in frontend/",
        )
            .into_response(),
    }
}

// ─── Health ──────────────────────────────────────────────────────

async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    // Quick check if deemix is authenticated (non-blocking)
    let deemix_authenticated = !state.config.deemix.arl.is_empty();

    Json(HealthResponse {
        status: "ok".to_string(),
        deemix_configured: !state.config.deemix.base_url.is_empty(),
        deemix_authenticated,
        spotify_configured: !state.config.spotify.client_id.is_empty(),
        spotdl_available: state.spotdl_available,
        ytdlp_available: state.ytdlp_available,
    })
}

// ─── Stats ───────────────────────────────────────────────────────

async fn stats(State(state): State<Arc<AppState>>) -> Result<Json<StatsResponse>, AppError> {
    let stats = db::get_stats(&state.pool).await?;
    Ok(Json(stats))
}

// ─── Queue ───────────────────────────────────────────────────────

async fn queue(State(state): State<Arc<AppState>>) -> Result<Json<QueueResponse>, AppError> {
    let submissions = db::get_submissions(&state.pool, None).await?;
    let stats = db::get_stats(&state.pool).await?;
    let tasks: Vec<SubmissionResponse> = submissions.into_iter().map(Into::into).collect();
    Ok(Json(QueueResponse {
        total: stats.total,
        ready: stats.ready,
        failed: stats.failed,
        pending: stats.pending,
        tasks,
    }))
}

// ─── Search ──────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default = "default_source")]
    source: String,
}

fn default_limit() -> u32 {
    5
}

fn default_source() -> String {
    "spotify".to_string()
}

async fn search(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, AppError> {
    let q = query.q.as_deref().unwrap_or("").trim().to_string();

    if q.len() < 2 {
        return Err(AppError::BadRequest(
            "Query must be at least 2 characters".to_string(),
        ));
    }

    let source = query.source.to_lowercase();
    let limit = query.limit.min(10);

    let results = match source.as_str() {
        "youtube" => {
            if !state.ytdlp_available {
                return Err(AppError::ServiceUnavailable(
                    "yt-dlp not available on PATH".to_string(),
                ));
            }
            crate::youtube::search_tracks(&q, limit)
                .await
                .map_err(|e| AppError::Internal(format!("YouTube search failed: {}", e)))?
        }
        "soundcloud" => {
            if !state.ytdlp_available {
                return Err(AppError::ServiceUnavailable(
                    "yt-dlp not available on PATH".to_string(),
                ));
            }
            crate::soundcloud::search_tracks(&q, limit)
                .await
                .map_err(|e| AppError::Internal(format!("SoundCloud search failed: {}", e)))?
        }
        _ => {
            // Default: Spotify
            let spotify = state.spotify.as_ref().ok_or_else(|| {
                AppError::ServiceUnavailable("Spotify not configured".to_string())
            })?;

            spotify
                .search_tracks(&q, limit)
                .await
                .map_err(|e| AppError::Internal(format!("Spotify search failed: {}", e)))?
        }
    };

    Ok(Json(SearchResponse {
        results,
        source: source.clone(),
    }))
}

// ─── Download ────────────────────────────────────────────────────

async fn download(
    State(state): State<Arc<AppState>>,
    Json(body): Json<DownloadRequest>,
) -> Result<Json<SubmissionResponse>, AppError> {
    let url = body.url.trim().to_string();

    // Detect source from URL or use provided source
    let source = body
        .source
        .as_deref()
        .map(|s| s.to_lowercase())
        .unwrap_or_else(|| detect_source(&url));

    // Reject unsupported URLs early with a clear error
    if source == "unknown" {
        return Err(AppError::BadRequest(
            "Unsupported URL. Please enter a Spotify, YouTube, or SoundCloud link.".to_string(),
        ));
    }

    // Validate URL format
    if !is_valid_url(&url, &source) {
        return Err(AppError::BadRequest(format!(
            "Invalid {} URL format",
            source
        )));
    }

    // Check for duplicate
    let existing = db::get_submission_by_url(&state.pool, &url).await?;
    if existing.is_some() {
        return Err(AppError::BadRequest(
            "This track has already been submitted".to_string(),
        ));
    }

    // Resolve track metadata
    let (title, artist, cover_url) = match source.as_str() {
        "spotify" => {
            if let Some(spotify) = &state.spotify {
                match spotify.get_track(&url).await {
                    Ok(Some(track)) => (Some(track.title), Some(track.artist), track.cover_url),
                    _ => (None, None, None),
                }
            } else {
                (None, None, None)
            }
        }
        _ => {
            // For youtube/soundcloud, try to get metadata via yt-dlp
            if state.ytdlp_available {
                match resolve_via_ytdlp(&url).await {
                    Ok(meta) => meta,
                    Err(e) => {
                        tracing::warn!("yt-dlp metadata resolution failed for {}: {e}", url);
                        (None, None, None)
                    }
                }
            } else {
                (None, None, None)
            }
        }
    };

    // Insert into DB
    let submission = db::insert_submission(
        &state.pool,
        &url,
        title.as_deref(),
        artist.as_deref(),
        cover_url.as_deref(),
        &source,
    )
    .await
    .map_err(|e| AppError::Internal(format!("Failed to create submission: {}", e)))?;

    // Notify the download worker
    state.download_notify.notify_one();

    tracing::info!(
        "Created submission {} for {} [{}] ({} - {})",
        submission.id,
        url,
        source,
        title.as_deref().unwrap_or("unknown"),
        artist.as_deref().unwrap_or("unknown"),
    );

    Ok(Json(submission.into()))
}

/// Detect the source platform from a URL.
/// Canonical source detection — delegates to models.rs to avoid duplication.
fn detect_source(url: &str) -> String {
    crate::models::detect_source(url)
}

/// Validate a URL is plausible for its source.
fn is_valid_url(url: &str, source: &str) -> bool {
    match source {
        "spotify" => crate::spotify::parse_spotify_track_id(url).is_some(),
        "youtube" => url.contains("youtube.com/watch") || url.contains("youtu.be/"),
        "soundcloud" => url.contains("soundcloud.com/"),
        _ => !url.is_empty(),
    }
}

/// Resolve track metadata (title, artist, cover) via yt-dlp.
async fn resolve_via_ytdlp(
    url: &str,
) -> anyhow::Result<(Option<String>, Option<String>, Option<String>)> {
    let output = tokio::process::Command::new("yt-dlp")
        .args(["--dump-json", "--no-playlist", "--skip-download", url])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim())?;

    let title = v["title"].as_str().map(|s| s.to_string());
    let artist = v["uploader"]
        .as_str()
        .or(v["channel"].as_str())
        .map(|s| s.to_string());
    let cover = v["thumbnail"].as_str().map(|s| s.to_string());

    Ok((title, artist, cover))
}

// ─── Tracks (Deck Feeder) ────────────────────────────────────────

async fn tracks(State(state): State<Arc<AppState>>) -> Result<Json<Vec<TrackItem>>, AppError> {
    let submissions = db::get_downloaded_submissions(&state.pool).await?;

    // Build a set of known filenames from the DB
    let mut db_files: std::collections::HashSet<String> = submissions
        .iter()
        .filter_map(|s| s.filename.clone())
        .collect();

    let mut items = Vec::new();

    // First: files known in the DB
    for sub in &submissions {
        if let Some(filename) = &sub.filename {
            let file_path = state.config.download.output_dir.join(filename);
            let size = tokio::fs::metadata(&file_path)
                .await
                .map(|m| m.len())
                .unwrap_or(0);

            let encoded = urlencoding::encode(filename);
            let url = format!("/downloads/{}", encoded);

            items.push(TrackItem {
                filename: filename.clone(),
                size,
                url,
                ready: sub.status == "ready",
            });
        }
    }

    // Then: orphaned files on disk not yet in DB
    if let Ok(mut entries) = tokio::fs::read_dir(&state.config.download.output_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let filename = entry.file_name().to_string_lossy().to_string();
            if !filename.ends_with(".mp3")
                && !filename.ends_with(".flac")
                && !filename.ends_with(".m4a")
            {
                continue;
            }
            if db_files.contains(&filename) {
                continue; // already listed above
            }
            if let Ok(meta) = entry.metadata().await {
                let encoded = urlencoding::encode(&filename);
                items.push(TrackItem {
                    filename: filename.clone(),
                    size: meta.len(),
                    url: format!("/downloads/{}", encoded),
                    ready: true,
                });
            }
        }
    }

    Ok(Json(items))
}

// ─── Serve Downloads ─────────────────────────────────────────────

async fn serve_download(
    State(state): State<Arc<AppState>>,
    Path(filename): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // Decode URL-encoded filename
    let decoded = urlencoding::decode(&filename)
        .map_err(|_| AppError::BadRequest("Invalid filename encoding".to_string()))?;

    let filename_str = decoded.as_ref();

    // Security: prevent path traversal
    if filename_str.contains("..") || filename_str.contains('/') || filename_str.contains('\\') {
        return Err(AppError::BadRequest("Invalid filename".to_string()));
    }

    // Verify the file is in the submissions table
    let submissions = db::get_downloaded_submissions(&state.pool).await?;
    let matched = submissions
        .iter()
        .any(|s| s.filename.as_deref() == Some(filename_str));

    if !matched {
        return Err(AppError::NotFound("File not found".to_string()));
    }

    let file_path = state.config.download.output_dir.join(filename_str);

    if !file_path.exists() {
        return Err(AppError::NotFound("File not found on disk".to_string()));
    }

    use tokio::io::AsyncSeekExt;
    use tokio_util::io::ReaderStream;

    let file = tokio::fs::File::open(&file_path)
        .await
        .map_err(|_| AppError::NotFound("Failed to read file".to_string()))?;

    let metadata = file
        .metadata()
        .await
        .map_err(|_| AppError::Internal("Failed to get file metadata".to_string()))?;
    let file_size = metadata.len();

    let mime = mime_guess::from_path(&file_path)
        .first_or_octet_stream()
        .to_string();

    // Support Range header for audio streaming
    if let Some(range_header) = headers.get(header::RANGE) {
        if let Ok(range_str) = range_header.to_str() {
            if let Some(range) = parse_range(range_str, file_size) {
                let start = range.0;
                let end = range.1;
                let mut file = file;
                file.seek(tokio::io::SeekFrom::Start(start))
                    .await
                    .map_err(|_| AppError::Internal("Failed to seek file".to_string()))?;
                let limited = file.take(end - start + 1);
                let stream = ReaderStream::new(limited);
                let body = Body::from_stream(stream);

                return Ok(Response::builder()
                    .status(StatusCode::PARTIAL_CONTENT)
                    .header(header::CONTENT_TYPE, mime)
                    .header(header::CONTENT_LENGTH, (end - start + 1).to_string())
                    .header(
                        header::CONTENT_RANGE,
                        format!("bytes {}-{}/{}", start, end, file_size),
                    )
                    .header(header::ACCEPT_RANGES, "bytes")
                    .body(body)
                    .expect("infallible: response builder"));
            }
        }
    }

    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, file_size.to_string())
        .header(header::ACCEPT_RANGES, "bytes")
        .body(body)
        .expect("infallible: response builder"))
}

fn parse_range(range_str: &str, file_size: u64) -> Option<(u64, u64)> {
    let range_str = range_str.strip_prefix("bytes=")?;
    let parts: Vec<&str> = range_str.split('-').collect();
    if parts.len() != 2 {
        return None;
    }

    let start: u64 = if parts[0].is_empty() {
        0
    } else {
        parts[0].parse().ok()?
    };

    let end: u64 = if parts[1].is_empty() {
        file_size - 1
    } else {
        parts[1].parse().ok()?
    };

    if start > end || end >= file_size {
        return None;
    }

    Some((start, end))
}
