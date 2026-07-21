use anyhow::Context;
use sqlx::sqlite::SqlitePool;

use crate::models::{Playlist, StatsResponse, Submission};

/// Run all SQL migrations via sqlx::migrate! compile-time macro.
/// Migrations are timestamp-sorted, checksum-verified, and idempotent.
/// Just add a .sql file to migrations/ — no code changes needed.
pub async fn run_migrations(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("Migration failed")?;
    tracing::info!("Migrations applied successfully");
    Ok(())
}

pub async fn insert_submission(
    pool: &SqlitePool,
    spotify_url: &str,
    track_title: Option<&str>,
    track_artist: Option<&str>,
    cover_url: Option<&str>,
    source: &str,
) -> anyhow::Result<Submission> {
    sqlx::query_as::<_, Submission>(
        "INSERT INTO submissions (spotify_url, track_title, track_artist, cover_url, source, status) VALUES (?, ?, ?, ?, ?, 'pending') RETURNING *"
    )
    .bind(spotify_url).bind(track_title).bind(track_artist).bind(cover_url).bind(source)
    .fetch_one(pool).await.context("Failed to insert submission")
}

pub async fn get_submissions(
    pool: &SqlitePool,
    status_filter: Option<&str>,
) -> anyhow::Result<Vec<Submission>> {
    if let Some(status) = status_filter {
        sqlx::query_as::<_, Submission>(
            "SELECT * FROM submissions WHERE status = ? ORDER BY created_at DESC",
        )
        .bind(status)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, Submission>("SELECT * FROM submissions ORDER BY created_at DESC")
            .fetch_all(pool)
            .await
    }
    .context("Failed to fetch submissions")
}

pub async fn get_submission_by_id(
    pool: &SqlitePool,
    id: i64,
) -> anyhow::Result<Option<Submission>> {
    sqlx::query_as::<_, Submission>("SELECT * FROM submissions WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("Failed to fetch submission")
}

pub async fn get_submission_by_url(
    pool: &SqlitePool,
    url: &str,
) -> anyhow::Result<Option<Submission>> {
    sqlx::query_as::<_, Submission>("SELECT * FROM submissions WHERE spotify_url = ?")
        .bind(url)
        .fetch_optional(pool)
        .await
        .context("Failed to fetch submission by URL")
}

pub async fn update_submission_status(
    pool: &SqlitePool,
    id: i64,
    status: &str,
    filename: Option<&str>,
    file_size: Option<i64>,
    error: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE submissions SET status = ?, filename = COALESCE(?, filename), file_size = COALESCE(?, file_size), error_message = COALESCE(?, error_message), updated_at = unixepoch() WHERE id = ?"
    )
    .bind(status).bind(filename).bind(file_size).bind(error).bind(id)
    .execute(pool).await.context("Failed to update submission status")?;
    Ok(())
}

pub async fn get_stats(pool: &SqlitePool) -> anyhow::Result<StatsResponse> {
    let row: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*) as total, COALESCE(SUM(CASE WHEN status = 'ready' THEN 1 ELSE 0 END), 0) as ready, COALESCE(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END), 0) as failed, COALESCE(SUM(CASE WHEN status IN ('pending', 'stage2_deemix', 'stage3_spotdl') THEN 1 ELSE 0 END), 0) as pending, 0 as placeholder FROM submissions"
    ).fetch_one(pool).await.context("Failed to get stats")?;
    Ok(StatsResponse {
        total: row.0,
        ready: row.1,
        failed: row.2,
        pending: row.3,
    })
}

pub async fn get_downloaded_submissions(pool: &SqlitePool) -> anyhow::Result<Vec<Submission>> {
    sqlx::query_as::<_, Submission>(
        "SELECT * FROM submissions WHERE filename IS NOT NULL AND status = 'ready' ORDER BY created_at DESC"
    ).fetch_all(pool).await.context("Failed to fetch downloaded submissions")
}

pub async fn get_pending_submissions(pool: &SqlitePool) -> anyhow::Result<Vec<Submission>> {
    sqlx::query_as::<_, Submission>(
        "SELECT * FROM submissions WHERE status IN ('pending', 'stage2_deemix', 'stage3_spotdl') ORDER BY created_at ASC"
    ).fetch_all(pool).await.context("Failed to fetch pending submissions")
}

pub async fn append_attempt(
    pool: &SqlitePool,
    id: i64,
    layer: &str,
    ok: bool,
    filename: Option<&str>,
    bitrate: Option<&str>,
    container: Option<&str>,
    error: Option<&str>,
) -> anyhow::Result<()> {
    let entry = serde_json::json!({
        "ts": chrono::Utc::now().format("%H:%M:%S").to_string(),
        "layer": layer, "ok": ok, "file": filename,
        "bitrate": bitrate, "container": container, "error": error,
    });
    let entry_str = entry.to_string();

    // Use json() to parse the string as a JSON value, ensuring it's stored as an object,
    // not a string literal. Both branches must match to avoid mixed-format arrays.
    sqlx::query(
        "UPDATE submissions SET attempts_json = CASE WHEN attempts_json IS NULL THEN json_array(json(?1)) ELSE json_insert(attempts_json, '$[#]', json(?1)) END, bitrate = COALESCE(?2, bitrate), container = COALESCE(?3, container) WHERE id = ?4"
    )
    .bind(&entry_str).bind(bitrate).bind(container).bind(id)
    .execute(pool).await.context("Failed to append attempt")?;
    Ok(())
}

pub async fn append_playlist_attempt(
    pool: &SqlitePool,
    id: i64,
    ok: bool,
    track_count: i64,
    new_count: i64,
    error: Option<&str>,
) -> anyhow::Result<()> {
    let entry = serde_json::json!({
        "ts": chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        "ok": ok,
        "track_count": track_count,
        "new_count": new_count,
        "error": error,
    });
    let entry_str = entry.to_string();
    sqlx::query(
        "UPDATE playlists SET attempts_json = CASE WHEN attempts_json IS NULL THEN json_array(json(?1)) ELSE json_insert(attempts_json, '$[#]', json(?1)) END WHERE id = ?2"
    )
    .bind(&entry_str).bind(id)
    .execute(pool).await?;
    Ok(())
}

pub async fn update_track_metadata(
    pool: &SqlitePool,
    id: i64,
    title: &str,
    artist: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE submissions SET track_title = COALESCE(track_title, ?), track_artist = COALESCE(track_artist, ?), updated_at = unixepoch() WHERE id = ?"
    )
    .bind(title)
    .bind(artist)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

// ── Playlist CRUD ────────────────────────────────────────────────────────────────────

pub async fn insert_playlist(
    pool: &SqlitePool,
    url: &str,
    source: &str,
    title: Option<&str>,
) -> anyhow::Result<Playlist> {
    sqlx::query_as::<_, Playlist>(
        "INSERT INTO playlists (url, source, title) VALUES (?, ?, ?) RETURNING *",
    )
    .bind(url)
    .bind(source)
    .bind(title)
    .fetch_one(pool)
    .await
    .context("Failed to insert playlist")
}

pub async fn get_playlists(pool: &SqlitePool) -> anyhow::Result<Vec<Playlist>> {
    sqlx::query_as::<_, Playlist>("SELECT * FROM playlists ORDER BY created_at DESC")
        .fetch_all(pool)
        .await
        .context("Failed to fetch playlists")
}

pub async fn get_playlist_by_id(pool: &SqlitePool, id: i64) -> anyhow::Result<Option<Playlist>> {
    sqlx::query_as::<_, Playlist>("SELECT * FROM playlists WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("Failed to fetch playlist")
}

pub async fn delete_playlist(pool: &SqlitePool, id: i64) -> anyhow::Result<bool> {
    let result = sqlx::query("DELETE FROM playlists WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .context("Failed to delete playlist")?;
    Ok(result.rows_affected() > 0)
}

pub async fn update_playlist_sync(
    pool: &SqlitePool,
    id: i64,
    title: Option<&str>,
    track_count: i64,
    new_since_sync: i64,
    error: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE playlists SET title = COALESCE(?, title), track_count = ?, new_since_sync = ?, last_synced = unixepoch(), last_error = ? WHERE id = ?",
    )
    .bind(title)
    .bind(track_count)
    .bind(new_since_sync)
    .bind(error)
    .bind(id)
    .execute(pool)
    .await
    .context("Failed to update playlist sync")?;
    Ok(())
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
        run_migrations(&pool)
            .await
            .expect("Failed to run migrations");
        pool
    }

    #[tokio::test]
    async fn test_insert_and_get_submission() {
        let pool = setup_test_db().await;
        let sub = insert_submission(
            &pool,
            "spotify:track:test123",
            Some("Test Track"),
            Some("Test Artist"),
            Some("https://example.com/cover.jpg"),
            "spotify",
        )
        .await
        .expect("Failed to insert");
        assert_eq!(sub.spotify_url, "spotify:track:test123");
        assert_eq!(sub.status, "pending");
        let fetched = get_submission_by_id(&pool, sub.id)
            .await
            .expect("Failed to fetch")
            .expect("Should exist");
        assert_eq!(fetched.id, sub.id);
    }

    #[tokio::test]
    async fn test_get_stats() {
        let pool = setup_test_db().await;
        insert_submission(
            &pool,
            "spotify:track:a",
            Some("A"),
            Some("A"),
            None,
            "spotify",
        )
        .await
        .unwrap();
        let sub2 = insert_submission(
            &pool,
            "spotify:track:b",
            Some("B"),
            Some("B"),
            None,
            "spotify",
        )
        .await
        .unwrap();
        let sub3 = insert_submission(
            &pool,
            "spotify:track:c",
            Some("C"),
            Some("C"),
            None,
            "spotify",
        )
        .await
        .unwrap();
        update_submission_status(&pool, sub2.id, "ready", Some("b.mp3"), Some(1000), None)
            .await
            .unwrap();
        update_submission_status(&pool, sub3.id, "failed", None, None, Some("Download error"))
            .await
            .unwrap();
        let stats = get_stats(&pool).await.unwrap();
        assert_eq!(stats.total, 3);
        assert_eq!(stats.ready, 1);
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.pending, 1);
    }

    #[tokio::test]
    async fn test_update_submission_status() {
        let pool = setup_test_db().await;
        let sub = insert_submission(
            &pool,
            "spotify:track:x",
            Some("X"),
            Some("X"),
            None,
            "spotify",
        )
        .await
        .unwrap();
        update_submission_status(&pool, sub.id, "ready", Some("x.mp3"), Some(5000), None)
            .await
            .unwrap();
        let updated = get_submission_by_id(&pool, sub.id).await.unwrap().unwrap();
        assert_eq!(updated.status, "ready");
        assert_eq!(updated.filename.as_deref(), Some("x.mp3"));
        assert_eq!(updated.file_size, Some(5000));
    }

    #[tokio::test]
    async fn test_get_pending_submissions() {
        let pool = setup_test_db().await;
        let sub1 = insert_submission(
            &pool,
            "spotify:track:1",
            Some("1"),
            Some("1"),
            None,
            "spotify",
        )
        .await
        .unwrap();
        let sub2 = insert_submission(
            &pool,
            "spotify:track:2",
            Some("2"),
            Some("2"),
            None,
            "spotify",
        )
        .await
        .unwrap();
        update_submission_status(&pool, sub1.id, "ready", Some("1.mp3"), None, None)
            .await
            .unwrap();
        update_submission_status(&pool, sub2.id, "stage2_deemix", None, None, None)
            .await
            .unwrap();
        let pending = get_pending_submissions(&pool).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, sub2.id);
    }
}
