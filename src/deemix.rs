use anyhow::Context;
use reqwest::Client;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct DeemixQueueItem {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub track_count_total: Option<i64>,
    #[serde(default)]
    pub track_count_downloaded: Option<i64>,
    #[serde(default)]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DeemixClient {
    base_url: String,
    client: Client,
}

impl DeemixClient {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: Client::new(),
        }
    }

    /// Add a Spotify URL to the deemix download queue.
    pub async fn add_to_queue(&self, spotify_url: &str) -> anyhow::Result<()> {
        let url = format!("{}/api/addToQueue", self.base_url);

        let body = serde_json::json!({
            "url": spotify_url,
        });

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("Failed to POST to deemix: {}", url))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Deemix addToQueue returned {}: {}", status, text);
        }

        tracing::info!("Added to deemix queue: {}", spotify_url);
        Ok(())
    }

    /// Get the current deemix queue.
    pub async fn get_queue(&self) -> anyhow::Result<Vec<DeemixQueueItem>> {
        let url = format!("{}/api/getQueue", self.base_url);

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Failed to GET deemix queue: {}", url))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Deemix getQueue returned {}: {}", status, text);
        }

        let items: Vec<DeemixQueueItem> = resp
            .json()
            .await
            .context("Failed to parse deemix queue response")?;

        Ok(items)
    }

    /// Find an item in the queue by URL.
    pub async fn find_by_url(&self, url: &str) -> anyhow::Result<Option<DeemixQueueItem>> {
        let queue = self.get_queue().await?;
        Ok(queue.into_iter().find(|item| item.url == url))
    }

    /// Poll until the item is done (ready or failed), with a timeout.
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

            if let Some(item) = queue.into_iter().find(|item| item.url == url) {
                match item.status.as_str() {
                    "queued" | "downloading" | "converting" => {
                        tracing::debug!("Deemix status for {}: {}", url, item.status);
                        tokio::time::sleep(poll_interval).await;
                        continue;
                    }
                    _ => {
                        // Done (finished, failed, error, etc.)
                        return Ok(Some(item));
                    }
                }
            }

            // Item not found in queue anymore, may have been processed
            tokio::time::sleep(poll_interval).await;
        }
    }
}
