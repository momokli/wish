use anyhow::Context;
use rspotify::ClientCredsSpotify;
use rspotify::Credentials;
use rspotify::model::SearchResult as RSpotifySearchResult;
use rspotify::model::SearchType;
use rspotify::model::TrackId;
use rspotify::prelude::*;

use crate::models::SearchResult;

pub struct SpotifyClient {
    client: ClientCredsSpotify,
}

impl SpotifyClient {
    /// Create a new Spotify client using client credentials flow.
    pub async fn new(client_id: &str, client_secret: &str) -> anyhow::Result<Self> {
        let creds = Credentials::new(client_id, client_secret);
        let client = ClientCredsSpotify::new(creds);
        client
            .request_token()
            .await
            .context("Failed to authenticate with Spotify")?;
        tracing::info!("Spotify client authenticated successfully");
        Ok(Self { client })
    }

    /// Search tracks on Spotify.
    pub async fn search_tracks(
        &self,
        query: &str,
        limit: u32,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let result = self
            .client
            .search(query, SearchType::Track, None, None, Some(limit), None)
            .await
            .context("Spotify search failed")?;
        let tracks = match result {
            RSpotifySearchResult::Tracks(page) => page.items,
            _ => vec![],
        };
        let results: Vec<SearchResult> = tracks
            .into_iter()
            .filter_map(|track| {
                let track_id = track.id.as_ref()?;
                let spotify_url = track_id.uri();
                let artist = track
                    .artists
                    .first()
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| "Unknown Artist".to_string());
                let cover_url = track.album.images.first().map(|img| img.url.clone());
                let duration_ms = track.duration.num_milliseconds() as u32;
                Some(SearchResult {
                    title: track.name,
                    artist,
                    cover_url,
                    spotify_url: spotify_url.clone(),
                    source_url: spotify_url,
                    source: "spotify".to_string(),
                    duration_ms: Some(duration_ms),
                })
            })
            .collect();
        Ok(results)
    }

    /// Get track metadata by Spotify track ID.
    pub async fn get_track(&self, spotify_url: &str) -> anyhow::Result<Option<SearchResult>> {
        let track_id_str = match parse_spotify_track_id(spotify_url) {
            Some(id) => id,
            None => return Ok(None),
        };
        let track_id = TrackId::from_id(&track_id_str)
            .map_err(|e| anyhow::anyhow!("Invalid track ID: {e}"))?;
        let track = self
            .client
            .track(track_id, None)
            .await
            .context("Failed to get track metadata")?;
        let artist = track
            .artists
            .first()
            .map(|a| a.name.clone())
            .unwrap_or_else(|| "Unknown Artist".to_string());
        let cover_url = track.album.images.first().map(|img| img.url.clone());
        let spotify_url = track.id.as_ref().map(|id| id.uri()).unwrap_or_default();
        let duration_ms = track.duration.num_milliseconds() as u32;
        Ok(Some(SearchResult {
            title: track.name,
            artist,
            cover_url,
            spotify_url: spotify_url.clone(),
            source_url: spotify_url,
            source: "spotify".to_string(),
            duration_ms: Some(duration_ms),
        }))
    }
}

/// Extract the Spotify playlist ID from a URL or URI.
pub fn parse_spotify_playlist_id(url: &str) -> Option<String> {
    if let Some(id) = url.strip_prefix("spotify:playlist:") {
        return Some(id.to_string());
    }
    if let Some(id) = url
        .strip_prefix("https://open.spotify.com/playlist/")
        .or_else(|| url.strip_prefix("http://open.spotify.com/playlist/"))
    {
        return Some(id.split('?').next().unwrap_or(id).to_string());
    }
    None
}

/// Extract the Spotify track ID from a URL or URI.
pub fn parse_spotify_track_id(url: &str) -> Option<String> {
    if let Some(id) = url.strip_prefix("spotify:track:") {
        return Some(id.to_string());
    }
    if let Some(id) = url
        .strip_prefix("https://open.spotify.com/track/")
        .or_else(|| url.strip_prefix("http://open.spotify.com/track/"))
    {
        return Some(id.split('?').next().unwrap_or(id).to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_spotify_track_id_uri() {
        assert_eq!(
            parse_spotify_track_id("spotify:track:4cOdK2wGLETKBW3PvgPWqT"),
            Some("4cOdK2wGLETKBW3PvgPWqT".to_string())
        );
    }
    #[test]
    fn test_parse_spotify_track_id_url() {
        assert_eq!(
            parse_spotify_track_id("https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT"),
            Some("4cOdK2wGLETKBW3PvgPWqT".to_string())
        );
    }
    #[test]
    fn test_parse_spotify_track_id_url_with_query() {
        assert_eq!(
            parse_spotify_track_id("https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT?si=abc"),
            Some("4cOdK2wGLETKBW3PvgPWqT".to_string())
        );
    }
    #[test]
    fn test_parse_spotify_track_id_invalid() {
        assert_eq!(parse_spotify_track_id("not a spotify url"), None);
    }
    #[test]
    fn test_parse_spotify_playlist_id() {
        assert_eq!(
            parse_spotify_playlist_id("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M"),
            Some("37i9dQZF1DXcBWIGoYBM5M".to_string())
        );
    }
}
