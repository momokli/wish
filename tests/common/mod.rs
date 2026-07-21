use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

/// Create an in-memory SQLite database and run all migrations.
pub async fn create_test_db() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("Failed to create test DB");

    // Run all migrations via the canonical runner
    wish::db::run_migrations(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

/// Seed a submission with the given parameters and return it.
pub async fn seed_submission(
    pool: &SqlitePool,
    spotify_url: &str,
    track_title: &str,
    track_artist: &str,
    status: &str,
    filename: Option<&str>,
) -> wish::models::Submission {
    sqlx::query_as::<_, wish::models::Submission>(
        r#"INSERT INTO submissions (spotify_url, track_title, track_artist, source, status, filename)
           VALUES (?, ?, ?, 'spotify', ?, ?)
           RETURNING *"#,
    )
    .bind(spotify_url)
    .bind(track_title)
    .bind(track_artist)
    .bind(status)
    .bind(filename)
    .fetch_one(pool)
    .await
    .expect("Failed to seed submission")
}
