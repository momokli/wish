use std::sync::Arc;

use sqlx::sqlite::SqlitePool;
use tokio::sync::Notify;

use crate::db;
use crate::models::Playlist;
use crate::spotify::parse_spotify_playlist_id;

/// Detect the source platform from a URL.
///
/// Canonical source detection — delegates to models.rs to avoid duplication.
pub fn detect_source(url: &str) -> String {
    crate::models::detect_source(url)
}

/// Resolve a playlist URL to a title and list of track URLs.
///
/// For Spotify playlists, the official Web API is used (client credentials).
/// Falls back to yt-dlp if the API call fails (e.g. missing credentials).
/// YouTube and SoundCloud playlists always use yt-dlp.
pub async fn resolve_playlist(url: &str) -> anyhow::Result<(String, Vec<String>)> {
    let source = detect_source(url);
    if source == "spotify" {
        // Try Spotify API first (faster, no DRM issues)
        match resolve_spotify(url).await {
            Ok(result) => return Ok(result),
            Err(e) => tracing::warn!("Spotify API failed, falling back to yt-dlp: {e}"),
        }
    }
    ytdlp_resolve(url).await
}

/// Resolve a Spotify playlist via the official Web API using client credentials.
async fn resolve_spotify(playlist_url: &str) -> anyhow::Result<(String, Vec<String>)> {
    let id = parse_spotify_playlist_id(playlist_url)
        .ok_or_else(|| anyhow::anyhow!("Invalid Spotify playlist URL"))?;

    // Get a fresh token via client credentials.
    // Read credentials from env (already configured for rspotify).
    let client_id = std::env::var("WISH_SPOTIFY_CLIENT_ID")
        .or_else(|_| std::env::var("SPOTIFY_CLIENT_ID"))
        .unwrap_or_default();
    let client_secret = std::env::var("WISH_SPOTIFY_CLIENT_SECRET")
        .or_else(|_| std::env::var("SPOTIFY_CLIENT_SECRET"))
        .unwrap_or_default();

    let http = reqwest::Client::new();
    let token_resp = http
        .post("https://accounts.spotify.com/api/token")
        .form(&[("grant_type", "client_credentials")])
        .basic_auth(&client_id, Some(&client_secret))
        .send()
        .await?;
    let token_data: serde_json::Value = token_resp.json().await?;
    let access_token = token_data["access_token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Failed to get Spotify access token"))?;

    // Fetch playlist metadata (title only).
    let meta_url = format!("https://api.spotify.com/v1/playlists/{id}?fields=name");
    let meta: serde_json::Value = http
        .get(&meta_url)
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await?
        .json()
        .await?;
    let title = meta["name"].as_str().unwrap_or("Playlist").to_string();

    // Fetch tracks with pagination.
    let mut tracks = Vec::new();
    let mut next = Some(format!(
        "https://api.spotify.com/v1/playlists/{id}/tracks?fields=items(track(uri)),next&limit=100"
    ));

    while let Some(url) = next.take() {
        let page: serde_json::Value = http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?
            .json()
            .await?;

        if let Some(items) = page["items"].as_array() {
            for item in items {
                if let Some(uri) = item["track"]["uri"].as_str() {
                    tracks.push(uri.to_string());
                }
            }
        }
        next = page["next"].as_str().map(String::from);
    }

    Ok((title, tracks))
}

/// Actually run yt-dlp to resolve a playlist.
async fn ytdlp_resolve(url: &str) -> anyhow::Result<(String, Vec<String>)> {
    let output = tokio::process::Command::new("yt-dlp")
        .args([
            "--flat-playlist",
            "--dump-single-json",
            "--no-warnings",
            "--skip-download",
            url,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to run yt-dlp: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("yt-dlp: {}", sanitize_ytdlp_error(&stderr)));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .map_err(|e| anyhow::anyhow!("Failed to parse yt-dlp JSON: {e}"))?;

    // Extract playlist title from top-level "title"
    let playlist_title = v["title"].as_str().unwrap_or(url).to_string();

    // Extract track URLs from the "entries" array
    let mut track_urls = Vec::new();
    if let Some(entries) = v["entries"].as_array() {
        for entry in entries {
            if let Some(track_url) = entry["url"].as_str() {
                track_urls.push(track_url.to_string());
            }
        }
    }

    Ok((playlist_title, track_urls))
}

/// Extract a short, meaningful error message from yt-dlp stderr.
fn sanitize_ytdlp_error(stderr: &str) -> String {
    // Find the LAST line starting with "ERROR:"
    let error_line = stderr
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with("ERROR:"));
    if let Some(line) = error_line {
        let short = line.trim();
        if short.len() > 200 {
            format!("{}...", &short[..197])
        } else {
            short.to_string()
        }
    } else {
        // Fallback: last non-empty line
        let last = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("Unknown yt-dlp error");
        if last.len() > 200 {
            format!("{}...", &last[..197])
        } else {
            last.to_string()
        }
    }
}

/// Sync a single playlist: resolve tracks, insert new ones into the submissions
/// table, update sync metadata, and notify the download worker.
///
/// # Logic
///
/// 1. Call `resolve_playlist(url)` to get the current list of track URLs.
/// 2. For each track URL, check if it already exists in `submissions` via
///    `get_submission_by_url`. If not, insert it with `insert_submission`.
/// 3. Count how many new tracks were inserted.
/// 4. Update the playlist's `track_count`, `new_since_sync`, and `last_synced`
///    via `update_playlist_sync`.
/// 5. If tracks were added, notify the download worker to pick them up.
/// 6. On error, record the error message in the playlist's `last_error` field.
pub async fn sync_one(
    pool: &SqlitePool,
    playlist: &Playlist,
    notify: &Arc<Notify>,
    spotify: Option<&crate::spotify::SpotifyClient>,
) -> anyhow::Result<()> {
    let id = playlist.id;
    tracing::info!("[{id}] Syncing playlist: {}", playlist.url);

    let (title, track_urls) = match resolve_playlist(&playlist.url).await {
        Ok(result) => result,
        Err(e) => {
            let err_msg = format!("Resolve failed: {e}");
            tracing::warn!("[{id}] {err_msg}");
            db::update_playlist_sync(pool, id, None, 0, 0, Some(&err_msg)).await?;
            let _ = db::append_playlist_attempt(pool, id, false, 0, 0, Some(&err_msg)).await;
            return Err(anyhow::anyhow!("{err_msg}"));
        }
    };

    let total = track_urls.len() as i64;
    let mut new_count = 0i64;

    for track_url in &track_urls {
        // Skip tracks already in the submissions table
        let existing = db::get_submission_by_url(pool, track_url).await?;
        if existing.is_some() {
            continue;
        }

        // Resolve metadata for Spotify tracks if client available
        let (title, artist, cover) = if playlist.source == "spotify" {
            if let Some(client) = spotify {
                match client.get_track(track_url).await {
                    Ok(Some(track)) => (Some(track.title), Some(track.artist), track.cover_url),
                    _ => (None, None, None),
                }
            } else {
                (None, None, None)
            }
        } else {
            (None, None, None)
        };

        match db::insert_submission(
            pool,
            track_url,
            title.as_deref(),
            artist.as_deref(),
            cover.as_deref(),
            &playlist.source,
            None, // isrc not available for playlist syncs
        )
        .await
        {
            Ok(_) => {
                new_count += 1;
            }
            Err(e) => {
                // Duplicate or any other transient error — skip silently.
                tracing::debug!("[{id}] Skipping track (insert failed): {track_url}: {e}");
            }
        }
    }

    // Update playlist with sync results
    db::update_playlist_sync(pool, id, Some(&title), total, new_count, None).await?;
    let _ = db::append_playlist_attempt(pool, id, true, total, new_count, None).await;

    tracing::info!("[{id}] Synced '{title}': {new_count}/{total} new tracks");

    // Wake the download worker if we added work
    if new_count > 0 {
        notify.notify_one();
    }

    Ok(())
}

/// Start a background task that periodically syncs all subscribed playlists.
///
/// The task loops forever, sleeping for `interval_minutes` between syncs.
/// On each cycle it fetches all playlists from the DB and syncs each one
/// sequentially, logging results and continuing on individual failures.
pub fn start_auto_sync(pool: SqlitePool, notify: Arc<Notify>, interval_minutes: u64) {
    tokio::spawn(async move {
        let interval = std::time::Duration::from_secs(interval_minutes * 60);
        tracing::info!("Playlist auto-sync started (interval: {interval_minutes}m)");

        loop {
            tokio::time::sleep(interval).await;

            let playlists = match db::get_playlists(&pool).await {
                Ok(list) => list,
                Err(e) => {
                    tracing::error!("Playlist auto-sync: failed to fetch playlists: {e}");
                    continue;
                }
            };

            if playlists.is_empty() {
                tracing::debug!("Playlist auto-sync: no playlists to sync");
                continue;
            }

            tracing::info!(
                "Playlist auto-sync: syncing {} playlist(s)",
                playlists.len()
            );

            for playlist in &playlists {
                if let Err(e) = sync_one(&pool, playlist, &notify, None).await {
                    tracing::warn!("[{}] Playlist auto-sync failed: {e}", playlist.id);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn setup_test_db() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("Failed to create test DB");
        db::run_migrations(&pool)
            .await
            .expect("Failed to run migrations");
        pool
    }

    #[test]
    fn test_detect_source_spotify_uri() {
        assert_eq!(
            detect_source("spotify:track:4cOdK2wGLETKBW3PvgPWqT"),
            "spotify"
        );
    }

    #[test]
    fn test_detect_source_spotify_url() {
        assert_eq!(
            detect_source("https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT"),
            "spotify"
        );
    }

    #[test]
    fn test_detect_source_youtube() {
        assert_eq!(
            detect_source("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            "youtube"
        );
        assert_eq!(detect_source("https://youtu.be/dQw4w9WgXcQ"), "youtube");
    }

    #[test]
    fn test_detect_source_soundcloud() {
        assert_eq!(
            detect_source("https://soundcloud.com/artist/track"),
            "soundcloud"
        );
    }

    #[test]
    fn test_detect_source_fallback() {
        assert_eq!(detect_source("https://example.com/something"), "unknown");
    }

    #[tokio::test]
    async fn test_sync_one_no_ytdlp() {
        let pool = setup_test_db().await;
        let notify = Arc::new(Notify::new());

        // Insert a playlist
        let playlist = db::insert_playlist(
            &pool,
            "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M",
            "spotify",
            Some("Today's Top Hits"),
        )
        .await
        .expect("Failed to insert playlist");

        // Without yt-dlp available, resolve_playlist should fail.
        // The error should be recorded in the playlist's last_error field.
        let result = sync_one(&pool, &playlist, &notify, None).await;
        assert!(result.is_err(), "sync_one should fail without yt-dlp");

        // Verify the error was recorded
        let updated = db::get_playlist_by_id(&pool, playlist.id)
            .await
            .expect("Failed to fetch playlist")
            .expect("Playlist should exist");
        assert!(
            updated.last_error.is_some(),
            "last_error should be set on failure"
        );
        assert!(
            updated.last_error.as_deref().unwrap().contains("yt-dlp"),
            "error should mention yt-dlp: {:?}",
            updated.last_error
        );
    }

    #[tokio::test]
    async fn test_sync_one_records_track_count_on_failure() {
        let pool = setup_test_db().await;
        let notify = Arc::new(Notify::new());

        let playlist = db::insert_playlist(
            &pool,
            "https://soundcloud.com/artist/set",
            "soundcloud",
            None,
        )
        .await
        .expect("Failed to insert playlist");

        let _ = sync_one(&pool, &playlist, &notify, None).await;

        let updated = db::get_playlist_by_id(&pool, playlist.id)
            .await
            .expect("Failed to fetch")
            .expect("Should exist");
        // Even on error, update_playlist_sync was called with track_count=0
        assert_eq!(updated.track_count, Some(0));
    }
}
