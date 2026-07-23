use serde::{Deserialize, Serialize};

/// Standard JSON wrapper for API responses.
#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub data: T,
}

/// A submission in the database.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Submission {
    pub id: i64,
    pub spotify_url: String,
    pub track_title: Option<String>,
    pub track_artist: Option<String>,
    pub cover_url: Option<String>,
    pub source: String,
    pub status: String,
    pub filename: Option<String>,
    pub file_size: Option<i64>,
    pub error_message: Option<String>,
    pub bitrate: Option<String>,
    pub container: Option<String>,
    pub attempts_json: Option<String>,
    pub isrc: Option<String>,
    pub deemix_queue_id: Option<String>,
    pub deezer_track_id: Option<i64>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub first_available_at: Option<i64>,
}

/// Submission response sent to the frontend (subset of fields).
/// Field names match the Python prototype for compatibility.
#[derive(Debug, Serialize)]
pub struct SubmissionResponse {
    pub id: i64,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub cover_url: Option<String>,
    pub spotify_url: String,
    pub source: String,
    pub status: String,
    pub filename: Option<String>,
    pub file_size: Option<i64>,
    pub error_message: Option<String>,
    pub created_at: Option<i64>,
    pub first_available_at: Option<i64>,
}

impl From<Submission> for SubmissionResponse {
    fn from(s: Submission) -> Self {
        Self {
            id: s.id,
            title: s.track_title,
            artist: s.track_artist,
            cover_url: s.cover_url,
            spotify_url: s.spotify_url,
            source: s.source,
            status: s.status,
            filename: s.filename,
            file_size: s.file_size,
            error_message: s.error_message,
            created_at: s.created_at,
            first_available_at: s.first_available_at,
        }
    }
}

/// Full admin view of a submission with all technical details.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AdminRow {
    pub id: i64,
    pub track_title: Option<String>,
    pub track_artist: Option<String>,
    pub spotify_url: String,
    pub source: String,
    pub status: String,
    pub filename: Option<String>,
    pub file_size: Option<i64>,
    pub error_message: Option<String>,
    pub bitrate: Option<String>,
    pub container: Option<String>,
    pub attempts_json: Option<String>,
    pub isrc: Option<String>,
    pub deemix_queue_id: Option<String>,
    pub deezer_track_id: Option<i64>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub first_available_at: Option<i64>,
}

/// A Spotify search result.
/// Field names use camelCase to match the Python prototype's frontend expectations.
#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub artist: String,
    #[serde(rename = "coverUrl")]
    pub cover_url: Option<String>,
    #[serde(rename = "spotifyUrl")]
    pub spotify_url: String,
    #[serde(rename = "sourceUrl")]
    pub source_url: String,
    pub source: String,
    #[serde(rename = "durationMs")]
    pub duration_ms: Option<u32>,
    #[serde(default)]
    pub isrc: Option<String>,
}

/// Stats response for GET /stats.
#[derive(Debug, Serialize)]
pub struct StatsResponse {
    pub total: i64,
    pub ready: i64,
    pub failed: i64,
    pub pending: i64,
}

/// Health check response.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub deemix_configured: bool,
    pub deemix_authenticated: bool,
    pub spotify_configured: bool,
    pub spotdl_available: bool,
    pub ytdlp_available: bool,
}

/// Queue response (matches Python prototype format).
#[derive(Debug, Serialize)]
pub struct QueueResponse {
    pub total: i64,
    pub ready: i64,
    pub failed: i64,
    pub pending: i64,
    pub tasks: Vec<SubmissionResponse>,
}

/// Search response wrapper.
#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub source: String,
}

/// Request body for POST /download.
#[derive(Debug, Deserialize)]
pub struct DownloadRequest {
    pub url: String,
    #[serde(default)]
    pub source: Option<String>,
}

/// A subscribed playlist in the database.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Playlist {
    pub id: i64,
    pub url: String,
    pub source: String,
    pub title: Option<String>,
    pub track_count: Option<i64>,
    pub new_since_sync: Option<i64>,
    pub last_synced: Option<i64>,
    pub last_error: Option<String>,
    pub attempts_json: Option<String>,
    pub created_at: Option<i64>,
}

/// Request body for POST /admin/playlists.
#[derive(Debug, Deserialize)]
pub struct AddPlaylistRequest {
    pub url: String,
    #[serde(default)]
    pub source: Option<String>,
}

/// Tracks response item for GET /tracks.
#[derive(Debug, Serialize)]
pub struct TrackItem {
    pub filename: String,
    pub size: u64,
    pub url: String,
    pub ready: bool,
}

/// Canonical source detection from a URL.
///
/// Used by both the download endpoint (api.rs) and playlist workflows
/// (playlists.rs). Kept here to avoid divergent implementations.
pub fn detect_source(url: &str) -> String {
    let lower = url.to_lowercase();
    if lower.contains("spotify.com") || lower.starts_with("spotify:") {
        "spotify".to_string()
    } else if lower.contains("youtube.com") || lower.contains("youtu.be") {
        "youtube".to_string()
    } else if lower.contains("soundcloud.com") {
        "soundcloud".to_string()
    } else {
        "unknown".to_string()
    }
}
