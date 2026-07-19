mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use sqlx::sqlite::SqlitePool;
use std::sync::Arc;
use tokio::sync::Notify;
use tower::util::ServiceExt;

use wish::api;

/// Create a test app with an in-memory database.
async fn test_app(pool: SqlitePool) -> Router {
    let config = wish::config::Config::default();
    let state = Arc::new(api::AppState {
        pool,
        config,
        spotify: None,
        download_notify: Arc::new(Notify::new()),
        ytdlp_available: false,
    });
    api::build_router(state)
}

async fn json_body(body: Body) -> Value {
    let bytes = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

#[tokio::test]
async fn health_returns_ok() {
    let pool = common::create_test_db().await;
    let app = test_app(pool).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response.into_body()).await;
    assert_eq!(body["status"], "ok");
    assert_eq!(body["spotify_configured"], false);
    assert_eq!(body["deemix_configured"], true); // default base_url is set
}

#[tokio::test]
async fn stats_starts_empty() {
    let pool = common::create_test_db().await;
    let app = test_app(pool).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/stats")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response.into_body()).await;
    assert_eq!(body["total"], 0);
    assert_eq!(body["ready"], 0);
    assert_eq!(body["failed"], 0);
    assert_eq!(body["pending"], 0);
}

#[tokio::test]
async fn queue_returns_submissions() {
    let pool = common::create_test_db().await;

    common::seed_submission(
        &pool,
        "spotify:track:aaa",
        "Track A",
        "Artist A",
        "pending",
        None,
    )
    .await;
    common::seed_submission(
        &pool,
        "spotify:track:bbb",
        "Track B",
        "Artist B",
        "ready",
        Some("b.mp3"),
    )
    .await;

    let app = test_app(pool).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/queue")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response.into_body()).await;
    let tasks = body["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 2);
}

#[tokio::test]
async fn download_invalid_url_returns_400() {
    let pool = common::create_test_db().await;
    let app = test_app(pool).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/download")
                .method("POST")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"url":"not-a-spotify-url"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn download_creates_submission() {
    let pool = common::create_test_db().await;
    let app = test_app(pool).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/download")
                .method("POST")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    r#"{"url":"spotify:track:4cOdK2wGLETKBW3PvgPWqT"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response.into_body()).await;
    assert_eq!(body["spotify_url"], "spotify:track:4cOdK2wGLETKBW3PvgPWqT");
    assert_eq!(body["status"], "pending");
    assert!(body["id"].as_i64().is_some());

    // Verify it appears in /queue
    let response = app
        .oneshot(
            Request::builder()
                .uri("/queue")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = json_body(response.into_body()).await;
    let tasks = body["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 1);
}

#[tokio::test]
async fn download_duplicate_returns_400() {
    let pool = common::create_test_db().await;
    let app = test_app(pool).await;

    // First request
    app.clone()
        .oneshot(
            Request::builder()
                .uri("/download")
                .method("POST")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    r#"{"url":"spotify:track:4cOdK2wGLETKBW3PvgPWqT"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Duplicate request
    let response = app
        .oneshot(
            Request::builder()
                .uri("/download")
                .method("POST")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    r#"{"url":"spotify:track:4cOdK2wGLETKBW3PvgPWqT"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn stats_counts_correct() {
    let pool = common::create_test_db().await;

    common::seed_submission(&pool, "spotify:track:a", "A", "A", "ready", Some("a.mp3")).await;
    common::seed_submission(&pool, "spotify:track:b", "B", "B", "pending", None).await;
    common::seed_submission(&pool, "spotify:track:c", "C", "C", "failed", None).await;

    let app = test_app(pool).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/stats")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response.into_body()).await;
    assert_eq!(body["total"], 3);
    assert_eq!(body["ready"], 1);
    assert_eq!(body["failed"], 1);
    assert_eq!(body["pending"], 1);
}

#[tokio::test]
async fn search_requires_query() {
    let pool = common::create_test_db().await;
    let app = test_app(pool).await;

    // No query parameter
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/search")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Too short query
    let response = app
        .oneshot(
            Request::builder()
                .uri("/search?q=a")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn tracks_returns_files() {
    let pool = common::create_test_db().await;

    common::seed_submission(
        &pool,
        "spotify:track:a",
        "Song A",
        "Artist A",
        "ready",
        Some("Song A - Artist A.mp3"),
    )
    .await;

    let app = test_app(pool).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/tracks")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response.into_body()).await;
    let tracks = body.as_array().unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0]["filename"], "Song A - Artist A.mp3");
    assert_eq!(tracks[0]["ready"], true);
}

#[tokio::test]
async fn downloads_404_unknown() {
    let pool = common::create_test_db().await;
    let app = test_app(pool).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/downloads/nonexistent.mp3")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn frontend_serves_html() {
    let pool = common::create_test_db().await;
    let app = test_app(pool).await;

    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(content_type.contains("text/html"));
}
