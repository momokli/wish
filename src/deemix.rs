use anyhow::Context;
use reqwest::Client;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Deserialize)]
pub struct DeemixQueueItem {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub track_count_total: Option<i64>,
    #[serde(default)]
    pub track_count_downloaded: Option<i64>,
    #[serde(default)]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct DeemixActionResult {
    #[serde(default)]
    result: bool,
    #[serde(default)]
    errid: Option<String>,
}

// ── Client ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DeemixClient {
    base_url: String,
    client: Client,
}

impl DeemixClient {
    /// Create a new DeemixClient with cookie-based session support.
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
    /// POST `/api/loginArl` — returns user info on success.
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
    /// Returns Ok(()) on `result: true`, fails otherwise.
    pub async fn add_to_queue(&self, url: &str) -> anyhow::Result<()> {
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

        let result: DeemixActionResult = serde_json::from_str(&text)
            .with_context(|| format!("Failed to parse addToQueue response: {}", text))?;

        if result.result {
            tracing::info!("Added to deemix queue: {}", url);
            Ok(())
        } else {
            anyhow::bail!(
                "Deemix addToQueue failed: {}",
                result.errid.as_deref().unwrap_or("unknown error")
            );
        }
    }

    /// Get the current deemix queue.
    pub async fn get_queue(&self) -> anyhow::Result<Vec<DeemixQueueItem>> {
        let resp = self
            .client
            .get(format!("{}/api/getQueue", self.base_url))
            .send()
            .await
            .context("Failed to GET getQueue")?;

        // Response is { "queue": { "uuid": { ... }, ... } }
        let text = resp.text().await.context("Failed to read getQueue body")?;
        let v: serde_json::Value =
            serde_json::from_str(&text).context("Failed to parse getQueue response")?;

        let items: Vec<DeemixQueueItem> = v
            .get("queue")
            .and_then(|q| q.as_object())
            .map(|obj| {
                obj.values()
                    .filter_map(|item| serde_json::from_value(item.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();

        Ok(items)
    }

    /// Find an item in the queue by URL.
    pub async fn find_by_url(&self, url: &str) -> anyhow::Result<Option<DeemixQueueItem>> {
        let queue = self.get_queue().await?;
        Ok(queue
            .into_iter()
            .find(|item| item.url.as_deref() == Some(url)))
    }

    /// Poll until the item is done, with timeout in seconds.
    pub async fn poll_until_done(
        &self,
        url: &str,
        timeout_secs: u64,
    ) -> anyhow::Result<Option<DeemixQueueItem>> {
        let start = std::time::Instant::now();
        let poll_interval = std::time::Duration::from_secs(2);

        loop {
            if start.elapsed().as_secs() > timeout_secs {
                anyhow::bail!("Timeout waiting for deemix to process: {}", url);
            }

            let queue = self.get_queue().await?;

            // Find by URL, or grab any terminal item as fallback
            let item = queue
                .iter()
                .find(|item| item.url.as_deref() == Some(url))
                .or_else(|| {
                    queue.iter().find(|i| {
                        !matches!(
                            i.status.as_str(),
                            "queued" | "downloading" | "converting" | "processing"
                        )
                    })
                })
                .cloned();

            if let Some(item) = item {
                match item.status.as_str() {
                    "queued" | "downloading" | "converting" | "processing" => {
                        tracing::debug!("Deemix status for {}: {}", url, item.status);
                        tokio::time::sleep(poll_interval).await;
                    }
                    _ => return Ok(Some(item)),
                }
            }

            tokio::time::sleep(poll_interval).await;
        }
    }
}
