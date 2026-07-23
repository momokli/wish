use anyhow::Context;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Models ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeemixLoginResponse {
    pub status: i64,
    pub arl: String,
    pub user: DeemixUser,
    pub childs: Vec<DeemixUser>,
    #[serde(default)]
    pub current_child: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeemixUser {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub can_stream_lossless: Option<bool>,
}

/// A single queue item in the deemix queue.
#[derive(Debug, Clone, Deserialize)]
pub struct DeemixQueueItem {
    #[serde(default)]
    pub uuid: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub progress: i64,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub downloaded: i64,
    #[serde(default)]
    pub failed: i64,
    #[serde(default)]
    pub errors: Vec<serde_json::Value>,
    #[serde(default)]
    pub files: Vec<DeemixFile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeemixFile {
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DeemixQueueObject {
    #[serde(default)]
    uuid: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DeemixAddToQueueResponse {
    #[serde(default)]
    result: bool,
    #[serde(default)]
    errid: Option<String>,
    #[serde(default)]
    data: Option<DeemixAddToQueueData>,
}

#[derive(Debug, Clone, Deserialize)]
struct DeemixAddToQueueData {
    #[serde(default)]
    obj: Vec<DeemixQueueObject>,
}

#[derive(Debug, Clone, Deserialize)]
struct DeemixActionResult {
    #[serde(default)]
    result: bool,
    #[serde(default)]
    errid: Option<String>,
}

/// Default interval (in seconds) between deemix queue polls.
pub const DEFAULT_POLL_INTERVAL_SECS: u64 = 2;

/// Status values that mean deemix is actively working and shouldn't be interrupted.
const ACTIVE_STATUSES: &[&str] = &["queued", "downloading", "processing", "converting"];

// ── Client ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DeemixClient {
    base_url: String,
    client: Client,
}

impl DeemixClient {
    pub fn new(base_url: String) -> Self {
        let client = reqwest::Client::builder()
            .cookie_store(true)
            .build()
            .expect("Failed to build reqwest client for DeemixClient");
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
        }
    }

    /// Authenticate with a Deezer ARL token.
    pub async fn login_arl(&self, arl: &str) -> anyhow::Result<DeemixLoginResponse> {
        let body = serde_json::json!({"status": 1, "arl": arl});
        let resp = self
            .client
            .post(format!("{}/api/loginArl", self.base_url))
            .json(&body)
            .send()
            .await
            .context("Failed to POST loginArl")?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("Deemix loginArl failed: {} {}", status, text);
        }
        serde_json::from_str(&text).context("Failed to parse loginArl response")
    }

    /// Add a URL to the deemix download queue.
    /// Returns the UUID if a fresh item was created, None if already queued.
    pub async fn add_to_queue(&self, url: &str) -> anyhow::Result<Option<String>> {
        let body = serde_json::json!({"url": url});
        let resp = self
            .client
            .post(format!("{}/api/addToQueue", self.base_url))
            .json(&body)
            .send()
            .await
            .with_context(|| "Failed to POST addToQueue")?;

        let text = resp
            .text()
            .await
            .context("Failed to read addToQueue body")?;

        if let Ok(full) = serde_json::from_str::<DeemixAddToQueueResponse>(&text) {
            if full.result {
                if let Some(uuid) = full
                    .data
                    .as_ref()
                    .and_then(|d| d.obj.first())
                    .map(|o| o.uuid.clone())
                {
                    if !uuid.is_empty() {
                        tracing::info!("Added to deemix queue: {} (uuid={})", url, uuid);
                        return Ok(Some(uuid));
                    }
                }
                tracing::info!("Added to deemix queue (already queued): {}", url);
                return Ok(None);
            }
            anyhow::bail!(
                "Deemix addToQueue failed: {}",
                full.errid.as_deref().unwrap_or("unknown error")
            );
        }

        let result: DeemixActionResult =
            serde_json::from_str(&text).with_context(|| format!("addToQueue parse: {text}"))?;
        if result.result {
            tracing::info!("Added to deemix queue: {}", url);
            Ok(None)
        } else {
            anyhow::bail!(
                "Deemix addToQueue failed: {}",
                result.errid.as_deref().unwrap_or("unknown error")
            );
        }
    }

    /// Retry a download by UUID. Use this when a track is already in the queue
    /// with a terminal status (completed/failed) — it re-downloads fresh.
    pub async fn retry_download(&self, uuid: &str) -> anyhow::Result<()> {
        let body = serde_json::json!({"uuid": uuid});
        let resp = self
            .client
            .post(format!("{}/api/retryDownload", self.base_url))
            .json(&body)
            .send()
            .await
            .with_context(|| format!("Failed to POST retryDownload for uuid={}", uuid))?;

        let text = resp
            .text()
            .await
            .context("Failed to read retryDownload body")?;
        let result: DeemixActionResult =
            serde_json::from_str(&text).with_context(|| format!("retryDownload parse: {text}"))?;

        if result.result {
            tracing::info!("Retried deemix download for uuid={}", uuid);
            Ok(())
        } else {
            anyhow::bail!(
                "Deemix retryDownload failed: {}",
                result.errid.as_deref().unwrap_or("unknown error")
            );
        }
    }

    /// Ensure a Spotify URL is queued for download, handling all states:
    /// - Already active (queued/downloading/processing): return uuid to poll
    /// - Already terminal (completed/failed): call retry_download, return uuid to poll
    /// - Not in queue: call add_to_queue, return new uuid
    ///
    /// Returns (uuid, is_fresh) — is_fresh means a new download was triggered.
    pub async fn ensure_queued(
        &self,
        url: &str,
        title: Option<&str>,
        artist: Option<&str>,
    ) -> anyhow::Result<(String, bool)> {
        let map = self.get_queue_map().await?;

        // Find by title/artist match (deemix doesn't return the original URL)
        let found = map.iter().find(|(_, item)| {
            let title_match = title
                .map(|t| item.title.to_lowercase().contains(&t.to_lowercase()))
                .unwrap_or(false);
            let artist_match = artist
                .map(|a| item.artist.to_lowercase().contains(&a.to_lowercase()))
                .unwrap_or(false);
            title_match && artist_match
        });

        if let Some((uuid, item)) = found {
            let status = item.status.to_lowercase();
            if ACTIVE_STATUSES.contains(&status.as_str()) {
                tracing::info!(
                    "Deemix already downloading: uuid={} title={} status={}",
                    uuid,
                    item.title,
                    item.status
                );
                return Ok((uuid.clone(), false));
            }
            // Terminal status — retry to re-download
            tracing::info!(
                "Deemix item terminal ({}), retrying: uuid={} title={}",
                item.status,
                uuid,
                item.title
            );
            self.retry_download(uuid).await?;
            return Ok((uuid.clone(), true));
        }

        // Not in queue — add fresh
        let new_uuid = self.add_to_queue(url).await?;
        match new_uuid {
            Some(uuid) => {
                tracing::info!("Fresh deemix download: uuid={}", uuid);
                Ok((uuid, true))
            }
            None => {
                // add_to_queue returned None (duplicate but not found by title match?)
                // Fall back: search again after add
                let map2 = self.get_queue_map().await?;
                let found2 = map2.iter().find(|(_, item)| {
                    let title_match = title
                        .map(|t| item.title.to_lowercase().contains(&t.to_lowercase()))
                        .unwrap_or(false);
                    let artist_match = artist
                        .map(|a| item.artist.to_lowercase().contains(&a.to_lowercase()))
                        .unwrap_or(false);
                    title_match && artist_match
                });
                match found2 {
                    Some((uuid, item)) => {
                        if ACTIVE_STATUSES.contains(&item.status.to_lowercase().as_str()) {
                            return Ok((uuid.clone(), false));
                        }
                        self.retry_download(uuid).await?;
                        Ok((uuid.clone(), true))
                    }
                    None => anyhow::bail!("deemix: queued but not found in queue by title match"),
                }
            }
        }
    }

    /// Get the full deemix queue, UUID-keyed.
    pub async fn get_queue_map(&self) -> anyhow::Result<HashMap<String, DeemixQueueItem>> {
        let resp = self
            .client
            .get(format!("{}/api/getQueue", self.base_url))
            .send()
            .await
            .context("Failed to GET getQueue")?;

        let text = resp.text().await.context("Failed to read getQueue body")?;
        let v: serde_json::Value =
            serde_json::from_str(&text).context("Failed to parse getQueue response")?;

        let mut map = HashMap::new();
        if let Some(queue) = v.get("queue").and_then(|q| q.as_object()) {
            for (uuid, item_json) in queue {
                if let Ok(mut item) = serde_json::from_value::<DeemixQueueItem>(item_json.clone()) {
                    item.uuid = uuid.clone();
                    map.insert(uuid.clone(), item);
                }
            }
        }
        Ok(map)
    }

    /// Poll until the item identified by UUID reaches a terminal status.
    pub async fn poll_by_uuid(
        &self,
        uuid: &str,
        timeout_secs: u64,
    ) -> anyhow::Result<Option<DeemixQueueItem>> {
        let start = std::time::Instant::now();
        let poll_interval = std::time::Duration::from_secs(DEFAULT_POLL_INTERVAL_SECS);

        loop {
            if start.elapsed().as_secs() > timeout_secs {
                anyhow::bail!("Timeout waiting for deemix to process uuid={}", uuid);
            }

            let map = self.get_queue_map().await?;

            if let Some(item) = map.get(uuid) {
                match item.status.as_str() {
                    "queued" | "downloading" | "converting" | "processing" => {
                        tracing::debug!(
                            "Deemix uuid={} status={} progress={}%",
                            uuid,
                            item.status,
                            item.progress
                        );
                        tokio::time::sleep(poll_interval).await;
                    }
                    status => {
                        tracing::info!("Deemix uuid={} finished with status={}", uuid, status);
                        return Ok(Some(item.clone()));
                    }
                }
            } else {
                tracing::debug!("Deemix uuid={} not yet in queue, waiting...", uuid);
                tokio::time::sleep(poll_interval).await;
            }
        }
    }
}
