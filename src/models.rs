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
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
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
        }
    }
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

/// Tracks response item for GET /tracks.
#[derive(Debug, Serialize)]
pub struct TrackItem {
    pub filename: String,
    pub size: u64,
    pub url: String,
    pub ready: bool,
}
