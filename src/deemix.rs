use anyhow::Context;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

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

/// Result from add_to_queue — both UUID (for polling) and Deezer track ID.
#[derive(Debug, Clone)]
pub struct DeemixEnqueueResult {
    pub uuid: String,
    pub deezer_track_id: Option<i64>,
}

/// The data.obj[0] field from the addToQueue response.
#[derive(Debug, Clone, Deserialize)]
struct DeemixQueueObject {
    #[serde(default)]
    uuid: String,
    #[serde(default)]
    id: Option<i64>,
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

// ── Client ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DeemixClient {
    base_url: String,
    client: Client,
    arl: String,
    auth_lock: Arc<Mutex<()>>,
}

impl DeemixClient {
    pub fn new(base_url: String, arl: String) -> Self {
        let client = reqwest::Client::builder()
            .cookie_store(true)
            .build()
            .expect("Failed to build reqwest client for DeemixClient");
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
            arl,
            auth_lock: Arc::new(Mutex::new(())),
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
    /// Returns DeemixEnqueueResult (uuid + deezer_track_id) if fresh, None if already queued.
    /// Auto-re-authenticates on NotLoggedIn errors.
    pub async fn add_to_queue(&self, url: &str) -> anyhow::Result<Option<DeemixEnqueueResult>> {
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
                        return Ok(Some(DeemixEnqueueResult {
                            uuid,
                            deezer_track_id: full
                                .data
                                .as_ref()
                                .and_then(|d| d.obj.first())
                                .and_then(|o| o.id),
                        }));
                    }
                }
                tracing::info!("Added to deemix queue (already queued): {}", url);
                return Ok(None);
            }
            // Check for NotLoggedIn — re-auth and retry once
            if full.errid.as_deref() == Some("NotLoggedIn") {
                // Serialize re-auth to prevent concurrent requests from
                // stomping each other's session cookies.
                let _guard = self.auth_lock.lock().await;
                tracing::warn!("Deemix session expired, re-authenticating...");
                self.login_arl(&self.arl).await?;
                // Retry once inline
                let retry_resp = self
                    .client
                    .post(format!("{}/api/addToQueue", self.base_url))
                    .json(&serde_json::json!({"url": url}))
                    .send()
                    .await
                    .context("Failed to retry addToQueue after re-auth")?;
                let retry_text = retry_resp
                    .text()
                    .await
                    .context("Failed to read retry addToQueue")?;
                if let Ok(full2) = serde_json::from_str::<DeemixAddToQueueResponse>(&retry_text) {
                    if full2.result {
                        let obj = full2.data.as_ref().and_then(|d| d.obj.first());
                        if let Some(o) = obj {
                            if !o.uuid.is_empty() {
                                return Ok(Some(DeemixEnqueueResult {
                                    uuid: o.uuid.clone(),
                                    deezer_track_id: o.id,
                                }));
                            }
                        }
                        return Ok(None);
                    }
                }
                anyhow::bail!("Deemix addToQueue failed after re-auth");
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

    /// Get the full deemix queue, UUID-keyed.
    ///
    /// When deemix session expires, `/api/getQueue` returns `{"queue":{}}`
    /// (empty) — not an error. Re-auths and retries once on empty response.
    pub async fn get_queue_map(&self) -> anyhow::Result<HashMap<String, DeemixQueueItem>> {
        for attempt in 0..2 {
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
                    if let Ok(mut item) =
                        serde_json::from_value::<DeemixQueueItem>(item_json.clone())
                    {
                        item.uuid = uuid.clone();
                        map.insert(uuid.clone(), item);
                    }
                }
            }

            if !map.is_empty() {
                return Ok(map);
            }

            if attempt == 0 {
                let _guard = self.auth_lock.lock().await;
                tracing::warn!("Deemix getQueue empty — re-authenticating");
                self.login_arl(&self.arl).await?;
            }
        }
        tracing::debug!("Deemix queue genuinely empty");
        Ok(HashMap::new())
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
