mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;

use std::sync::Arc;
use tokio::sync::Notify;
use tower::util::ServiceExt;

use wish::api;

// ── Helpers ──────────────────────────────────────────────────────────────────────────

/// Create a test app with an in-memory database that includes all migrations.
async fn test_app() -> Router {
    let pool = common::create_test_db().await;
    let config = wish::config::Config::default();
    let state = Arc::new(api::AppState {
        pool,
        config,
        spotify: None,
        download_notify: Arc::new(Notify::new()),
        ytdlp_available: false,
        spotdl_available: false,
    });
    api::build_router(state)
}

/// Read a response body as JSON.
async fn json_body(body: Body) -> Value {
    let bytes = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// Helper: POST `/playlists` and return the created playlist JSON.
async fn add_playlist(app: &Router, url: &str, source: Option<&str>) -> Value {
    let mut body = serde_json::json!({"url": url});
    if let Some(src) = source {
        body["source"] = serde_json::json!(src);
    }
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/playlists")
                .method("POST")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let value: Value = json_body(response.into_body()).await;
    assert_eq!(status, StatusCode::OK, "add_playlist failed: {value:?}");
    value
}

// ── Tests ────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn add_playlist_creates_playlist() {
    let app = test_app().await;

    let playlist = add_playlist(
        &app,
        "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M",
        Some("spotify"),
    )
    .await;

    assert!(
        playlist["id"].as_i64().is_some(),
        "response should have an id: {playlist:?}"
    );
    assert_eq!(playlist["source"], "spotify");
    assert_eq!(
        playlist["url"],
        "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M"
    );
}

#[tokio::test]
async fn add_playlist_detects_source() {
    let app = test_app().await;

    // No source provided — should auto-detect from URL
    let body = serde_json::json!({
        "url": "https://youtube.com/playlist?list=PLxyz"
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/playlists")
                .method("POST")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let playlist: Value = json_body(response.into_body()).await;

    assert_eq!(
        playlist["source"], "youtube",
        "source should be auto-detected as 'youtube': {playlist:?}"
    );
}

#[tokio::test]
async fn list_playlists_returns_playlists() {
    let app = test_app().await;

    // Add two playlists
    let _ = add_playlist(
        &app,
        "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M",
        Some("spotify"),
    )
    .await;
    let _ = add_playlist(
        &app,
        "https://youtube.com/playlist?list=PLxyz",
        Some("youtube"),
    )
    .await;

    // GET /playlists
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/playlists")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response.into_body()).await;
    let playlists = body.as_array().unwrap();

    assert_eq!(playlists.len(), 2, "should return 2 playlists");
}

#[tokio::test]
async fn delete_playlist_removes_it() {
    let app = test_app().await;

    // Add a playlist
    let playlist = add_playlist(
        &app,
        "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M",
        Some("spotify"),
    )
    .await;
    let id = playlist["id"].as_i64().unwrap();

    // DELETE it
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!("/playlists/{}", id))
                .method("DELETE")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "DELETE should return 200"
    );

    // Verify it's gone
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/playlists")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body: Value = json_body(response.into_body()).await;
    let playlists = body.as_array().unwrap();
    assert_eq!(playlists.len(), 0, "playlist should be removed");
}

#[tokio::test]
async fn delete_playlist_404() {
    let app = test_app().await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/playlists/999")
                .method("DELETE")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn sync_playlist_accepts_sync() {
    let app = test_app().await;

    // Add a playlist with a bogus URL — insertion itself succeeds,
    // but the initial sync (triggered inside add_playlist) will fail
    // because there's no yt-dlp in CI. Use a direct POST to insert
    // without triggering sync issues.
    let body = serde_json::json!({
        "url": "https://open.spotify.com/playlist/bogus-sync-test",
        "source": "spotify"
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/playlists")
                .method("POST")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let inserted: Value = json_body(response.into_body()).await;
    let id = inserted["id"].as_i64().expect("playlist should have an id");

    // POST /playlists/{id}/sync
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!("/playlists/{}/sync", id))
                .method("POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // The endpoint should return a response (sync may fail without yt-dlp).
    // Regardless of the HTTP status, the handler should have recorded an error
    // in the database.
    if response.status() == StatusCode::OK {
        let synced: Value = json_body(response.into_body()).await;
        assert_eq!(synced["id"], serde_json::json!(id));
    }

    // Verify that the playlist's last_error was set (by querying /playlists)
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/playlists")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body: Value = json_body(response.into_body()).await;
    let playlists = body.as_array().unwrap();
    let updated = playlists
        .iter()
        .find(|p| p["id"] == serde_json::json!(id))
        .expect("playlist should still exist");

    assert!(
        updated["last_error"].is_string(),
        "playlist should have a last_error set after sync attempt: {updated:?}"
    );
    let err_msg = updated["last_error"].as_str().unwrap();
    assert!(
        !err_msg.is_empty(),
        "last_error should not be empty: {updated:?}"
    );
}
