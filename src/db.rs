use anyhow::Context;
use sqlx::sqlite::SqlitePool;

use crate::models::{StatsResponse, Submission};

/// Run all SQL migrations in order.
pub async fn run_migrations(pool: &SqlitePool) -> anyhow::Result<()> {
    let migrations = [
        include_str!("../migrations/001_initial_schema.sql"),
        include_str!("../migrations/002_admin_fields.sql"),
    ];
    for m in migrations {
        sqlx::query(m)
            .execute(pool)
            .await
            .context("Failed to run migration")?;
    }
    tracing::info!("Migrations applied successfully");
    Ok(())
}

/// Insert a new submission and return it.
pub async fn insert_submission(
    pool: &SqlitePool,
    spotify_url: &str,
    track_title: Option<&str>,
    track_artist: Option<&str>,
    cover_url: Option<&str>,
    source: &str,
) -> anyhow::Result<Submission> {
    let submission = sqlx::query_as::<_, Submission>(
        r#"INSERT INTO submissions (spotify_url, track_title, track_artist, cover_url, source, status)
           VALUES (?, ?, ?, ?, ?, 'pending')
           RETURNING *"#,
    )
    .bind(spotify_url)
    .bind(track_title)
    .bind(track_artist)
    .bind(cover_url)
    .bind(source)
    .fetch_one(pool)
    .await
    .context("Failed to insert submission")?;

    Ok(submission)
}

/// Get all submissions, optionally filtered by status.
pub async fn get_submissions(
    pool: &SqlitePool,
    status_filter: Option<&str>,
) -> anyhow::Result<Vec<Submission>> {
    let submissions = if let Some(status) = status_filter {
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
    .context("Failed to fetch submissions")?;

    Ok(submissions)
}

/// Get a single submission by ID.
pub async fn get_submission_by_id(
    pool: &SqlitePool,
    id: i64,
) -> anyhow::Result<Option<Submission>> {
    let submission = sqlx::query_as::<_, Submission>("SELECT * FROM submissions WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("Failed to fetch submission")?;

    Ok(submission)
}

/// Get submission by Spotify URL.
pub async fn get_submission_by_url(
    pool: &SqlitePool,
    url: &str,
) -> anyhow::Result<Option<Submission>> {
    let submission =
        sqlx::query_as::<_, Submission>("SELECT * FROM submissions WHERE spotify_url = ?")
            .bind(url)
            .fetch_optional(pool)
            .await
            .context("Failed to fetch submission by URL")?;

    Ok(submission)
}

/// Update a submission's status, filename, file_size, and error message.
pub async fn update_submission_status(
    pool: &SqlitePool,
    id: i64,
    status: &str,
    filename: Option<&str>,
    file_size: Option<i64>,
    error: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"UPDATE submissions
           SET status = ?, filename = COALESCE(?, filename),
               file_size = COALESCE(?, file_size),
               error_message = COALESCE(?, error_message),
               updated_at = unixepoch()
           WHERE id = ?"#,
    )
    .bind(status)
    .bind(filename)
    .bind(file_size)
    .bind(error)
    .bind(id)
    .execute(pool)
    .await
    .context("Failed to update submission status")?;

    Ok(())
}

/// Get stats (counts by status).
pub async fn get_stats(pool: &SqlitePool) -> anyhow::Result<StatsResponse> {
    let row: (i64, i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
               COUNT(*) as total,
               COALESCE(SUM(CASE WHEN status = 'ready' THEN 1 ELSE 0 END), 0) as ready,
               COALESCE(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END), 0) as failed,
               COALESCE(SUM(CASE WHEN status IN ('pending', 'stage2_deemix', 'stage3_spotdl') THEN 1 ELSE 0 END), 0) as pending,
               0 as placeholder
           FROM submissions"#,
    )
    .fetch_one(pool)
    .await
    .context("Failed to get stats")?;

    Ok(StatsResponse {
        total: row.0,
        ready: row.1,
        failed: row.2,
        pending: row.3,
    })
}

/// Get all submissions that have a filename set (ready or otherwise).
pub async fn get_downloaded_submissions(pool: &SqlitePool) -> anyhow::Result<Vec<Submission>> {
    let submissions = sqlx::query_as::<_, Submission>(
        "SELECT * FROM submissions WHERE filename IS NOT NULL AND status = 'ready' ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await
    .context("Failed to fetch downloaded submissions")?;

    Ok(submissions)
}

/// Get all pending submissions that need processing.
pub async fn get_pending_submissions(pool: &SqlitePool) -> anyhow::Result<Vec<Submission>> {
    let submissions = sqlx::query_as::<_, Submission>(
        "SELECT * FROM submissions WHERE status IN ('pending', 'stage2_deemix', 'stage3_spotdl') ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await
    .context("Failed to fetch pending submissions")?;

    Ok(submissions)
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
        assert_eq!(sub.track_title.as_deref(), Some("Test Track"));

        let fetched = get_submission_by_id(&pool, sub.id)
            .await
            .expect("Failed to fetch")
            .expect("Should exist");
        assert_eq!(fetched.id, sub.id);
    }

    #[tokio::test]
    async fn test_get_stats() {
        let pool = setup_test_db().await;

        // Insert submissions with different statuses
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

        // Update statuses
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
