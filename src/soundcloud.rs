use crate::models::SearchResult;
use anyhow::Context;

/// Search SoundCloud using yt-dlp (no API key needed).
pub async fn search_tracks(query: &str, limit: u32) -> anyhow::Result<Vec<SearchResult>> {
    let search_query = format!("scsearch{}:{}", limit.min(10), query);

    let output = tokio::process::Command::new("yt-dlp")
        .args([
            "--flat-playlist",
            "--dump-json",
            "--no-playlist",
            "--skip-download",
            "--socket-timeout",
            "10",
            &search_query,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .context("Failed to run yt-dlp for SoundCloud search")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "yt-dlp SoundCloud search failed: {}",
            stderr
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let results: Vec<SearchResult> = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).ok()?;
            let id = v["id"].as_str().unwrap_or("");
            let title = v["title"].as_str().unwrap_or("Unknown Title").to_string();
            let artist = v["uploader"]
                .as_str()
                .or(v["channel"].as_str())
                .unwrap_or("Unknown Artist")
                .to_string();
            let cover_url = v["thumbnail"].as_str().map(|s| s.to_string());
            let duration = v["duration"].as_f64().map(|d| (d * 1000.0) as u32);
            let url = v["webpage_url"]
                .as_str()
                .or(v["url"].as_str())
                .unwrap_or(&format!("https://soundcloud.com/{}", id))
                .to_string();

            Some(SearchResult {
                title,
                artist,
                cover_url,
                spotify_url: url.clone(),
                source_url: url,
                source: "soundcloud".to_string(),
                duration_ms: duration,
                isrc: None,
            })
        })
        .take(limit as usize)
        .collect();

    Ok(results)
}
