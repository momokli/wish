use sqlx::sqlite::SqlitePool;
use std::path::PathBuf;
use tokio::sync::Notify;

use crate::db;
use crate::deemix::DeemixClient;

/// Background worker that processes pending download submissions.
pub struct DownloadWorker {
    pool: SqlitePool,
    deemix: DeemixClient,
    output_dir: PathBuf,
    notify: std::sync::Arc<Notify>,
    ytdlp_available: bool,
}

impl DownloadWorker {
    pub fn new(
        pool: SqlitePool,
        deemix: DeemixClient,
        output_dir: PathBuf,
        notify: std::sync::Arc<Notify>,
        ytdlp_available: bool,
    ) -> Self {
        Self {
            pool,
            deemix,
            output_dir,
            notify,
            ytdlp_available,
        }
    }

    /// Start the background worker loop.
    pub async fn run(self) {
        tracing::info!("Download worker started (yt-dlp: {})", self.ytdlp_available);

        loop {
            tokio::select! {
                _ = self.notify.notified() => {
                    tracing::debug!("Download worker notified");
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                    // Periodic check
                }
            }

            if let Err(e) = self.process_pending().await {
                tracing::error!("Error processing pending submissions: {}", e);
            }
        }
    }

    async fn process_pending(&self) -> anyhow::Result<()> {
        let pending = db::get_pending_submissions(&self.pool).await?;

        if pending.is_empty() {
            return Ok(());
        }

        tracing::info!("Processing {} pending submission(s)", pending.len());

        for submission in pending {
            if let Err(e) = self.process_submission(&submission).await {
                tracing::error!(
                    "Failed to process submission {} ({}): {}",
                    submission.id,
                    submission.spotify_url,
                    e
                );
                let _ = db::update_submission_status(
                    &self.pool,
                    submission.id,
                    "failed",
                    None,
                    None,
                    Some(&e.to_string()),
                )
                .await;
            }
        }

        Ok(())
    }

    async fn process_submission(
        &self,
        submission: &crate::models::Submission,
    ) -> anyhow::Result<()> {
        let source = submission.source.as_str();

        tracing::info!(
            "Processing submission {} [{}]: {}",
            submission.id,
            source,
            submission.spotify_url
        );

        match source {
            "youtube" | "soundcloud" => {
                self.download_via_ytdlp(submission).await?;
            }
            _ => {
                // Spotify: existing deemix → spotDL → yt-dlp pipeline
                self.download_spotify(submission).await?;
            }
        }

        Ok(())
    }

    /// Download YouTube or SoundCloud track directly via yt-dlp.
    async fn download_via_ytdlp(
        &self,
        submission: &crate::models::Submission,
    ) -> anyhow::Result<()> {
        if !self.ytdlp_available {
            return Err(anyhow::anyhow!("yt-dlp not available on PATH"));
        }

        db::update_submission_status(&self.pool, submission.id, "stage2_deemix", None, None, None)
            .await?;

        let output_template = self
            .output_dir
            .join("%(title)s-%(id)s.%(ext)s")
            .to_string_lossy()
            .to_string();

        tracing::info!(
            "Downloading via yt-dlp: {} -> {}",
            submission.spotify_url,
            output_template
        );

        let result = tokio::process::Command::new("yt-dlp")
            .args([
                "-f",
                "bestaudio",
                "--extract-audio",
                "--audio-format",
                "mp3",
                "--audio-quality",
                "0",
                "-o",
                &output_template,
                "--no-playlist",
                &submission.spotify_url,
            ])
            .output()
            .await;

        match result {
            Ok(output) => {
                if output.status.success() {
                    if let Some(filename) = self.find_downloaded_file(submission).await {
                        let file_size = std::fs::metadata(self.output_dir.join(&filename))
                            .ok()
                            .map(|m| m.len() as i64);

                        db::update_submission_status(
                            &self.pool,
                            submission.id,
                            "ready",
                            Some(&filename),
                            file_size,
                            None,
                        )
                        .await?;

                        tracing::info!(
                            "Submission {} downloaded via yt-dlp: {}",
                            submission.id,
                            filename
                        );
                    } else {
                        db::update_submission_status(
                            &self.pool,
                            submission.id,
                            "failed",
                            None,
                            None,
                            Some("yt-dlp succeeded but couldn't find output file"),
                        )
                        .await?;
                    }
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    db::update_submission_status(
                        &self.pool,
                        submission.id,
                        "failed",
                        None,
                        None,
                        Some(&format!("yt-dlp failed: {}", stderr)),
                    )
                    .await?;
                }
            }
            Err(e) => {
                db::update_submission_status(
                    &self.pool,
                    submission.id,
                    "failed",
                    None,
                    None,
                    Some(&format!("yt-dlp command error: {}", e)),
                )
                .await?;
            }
        }

        Ok(())
    }

    /// Original Spotify pipeline: deemix → spotDL → yt-dlp fallback.
    async fn download_spotify(&self, submission: &crate::models::Submission) -> anyhow::Result<()> {
        // Stage 2: Try deemix
        db::update_submission_status(&self.pool, submission.id, "stage2_deemix", None, None, None)
            .await?;

        let deemix_result = self.deemix.add_to_queue(&submission.spotify_url).await;

        match deemix_result {
            Ok(()) => {
                match self
                    .deemix
                    .poll_until_done(&submission.spotify_url, 300)
                    .await
                {
                    Ok(Some(item)) => {
                        if item.status == "finished" || item.status == "downloaded" {
                            if let Some(filename) = self.find_downloaded_file(submission).await {
                                let file_size = std::fs::metadata(self.output_dir.join(&filename))
                                    .ok()
                                    .map(|m| m.len() as i64);

                                db::update_submission_status(
                                    &self.pool,
                                    submission.id,
                                    "ready",
                                    Some(&filename),
                                    file_size,
                                    None,
                                )
                                .await?;

                                tracing::info!(
                                    "Submission {} downloaded via deemix: {}",
                                    submission.id,
                                    filename
                                );
                                return Ok(());
                            }
                        }
                        tracing::warn!(
                            "Deemix returned status '{}' for submission {}",
                            item.status,
                            submission.id
                        );
                    }
                    Ok(None) => {
                        tracing::warn!(
                            "Deemix queue item not found for submission {}",
                            submission.id
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Deemix polling failed for submission {}: {}",
                            submission.id,
                            e
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Deemix addToQueue failed for submission {}: {}",
                    submission.id,
                    e
                );
            }
        }

        // Stage 3: spotDL fallback
        tracing::info!("Falling back to spotDL for submission {}", submission.id);

        db::update_submission_status(&self.pool, submission.id, "stage3_spotdl", None, None, None)
            .await?;

        if self.try_spotdl(submission).await {
            return Ok(());
        }

        // Stage 4: yt-dlp fallback (if available)
        if self.ytdlp_available {
            tracing::info!("Falling back to yt-dlp for submission {}", submission.id);

            db::update_submission_status(
                &self.pool,
                submission.id,
                "stage3_spotdl",
                None,
                None,
                None,
            )
            .await?;

            let search_query = if let (Some(title), Some(artist)) =
                (&submission.track_title, &submission.track_artist)
            {
                format!("ytsearch1:{} {}", artist, title)
            } else {
                return Err(anyhow::anyhow!(
                    "No track metadata available for yt-dlp search"
                ));
            };

            let output_template = self
                .output_dir
                .join("%(title)s-%(id)s.%(ext)s")
                .to_string_lossy()
                .to_string();

            let result = tokio::process::Command::new("yt-dlp")
                .args([
                    "-f",
                    "bestaudio",
                    "--extract-audio",
                    "--audio-format",
                    "mp3",
                    "-o",
                    &output_template,
                    &search_query,
                ])
                .output()
                .await;

            match result {
                Ok(output) => {
                    if output.status.success() {
                        if let Some(filename) = self.find_downloaded_file(submission).await {
                            let file_size = std::fs::metadata(self.output_dir.join(&filename))
                                .ok()
                                .map(|m| m.len() as i64);

                            db::update_submission_status(
                                &self.pool,
                                submission.id,
                                "ready",
                                Some(&filename),
                                file_size,
                                None,
                            )
                            .await?;

                            tracing::info!(
                                "Submission {} downloaded via yt-dlp (fallback): {}",
                                submission.id,
                                filename
                            );
                            return Ok(());
                        }
                    }
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    db::update_submission_status(
                        &self.pool,
                        submission.id,
                        "failed",
                        None,
                        None,
                        Some(&format!("yt-dlp fallback failed: {}", stderr)),
                    )
                    .await?;
                }
                Err(e) => {
                    db::update_submission_status(
                        &self.pool,
                        submission.id,
                        "failed",
                        None,
                        None,
                        Some(&format!("yt-dlp fallback error: {}", e)),
                    )
                    .await?;
                }
            }
        } else {
            db::update_submission_status(
                &self.pool,
                submission.id,
                "failed",
                None,
                None,
                Some("All download methods exhausted (deemix, spotDL, yt-dlp not available)"),
            )
            .await?;
        }

        Ok(())
    }

    /// Try spotDL download. Returns true on success.
    async fn try_spotdl(&self, submission: &crate::models::Submission) -> bool {
        let result = tokio::process::Command::new("spotdl")
            .arg("download")
            .arg(&submission.spotify_url)
            .arg("--output")
            .arg(&self.output_dir)
            .output()
            .await;

        match result {
            Ok(output) => {
                if output.status.success() {
                    if let Some(filename) = self.find_downloaded_file(submission).await {
                        let file_size = std::fs::metadata(self.output_dir.join(&filename))
                            .ok()
                            .map(|m| m.len() as i64);

                        let _ = db::update_submission_status(
                            &self.pool,
                            submission.id,
                            "ready",
                            Some(&filename),
                            file_size,
                            None,
                        )
                        .await;

                        tracing::info!(
                            "Submission {} downloaded via spotDL: {}",
                            submission.id,
                            filename
                        );
                        return true;
                    } else {
                        let _ = db::update_submission_status(
                            &self.pool,
                            submission.id,
                            "failed",
                            None,
                            None,
                            Some("spotDL succeeded but couldn't find output file"),
                        )
                        .await;
                        return false;
                    }
                }

                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::warn!("spotDL failed for {}: {}", submission.id, stderr);
                false
            }
            Err(e) => {
                tracing::warn!("spotDL command error for {}: {}", submission.id, e);
                false
            }
        }
    }

    /// Try to find a downloaded file for a submission.
    async fn find_downloaded_file(&self, submission: &crate::models::Submission) -> Option<String> {
        let mut entries = tokio::fs::read_dir(&self.output_dir).await.ok()?;

        let title = submission.track_title.as_deref().unwrap_or("");
        let artist = submission.track_artist.as_deref().unwrap_or("");

        while let Ok(Some(entry)) = entries.next_entry().await {
            let filename = entry.file_name();
            let name = filename.to_string_lossy().to_string();

            if !title.is_empty() && name.to_lowercase().contains(&title.to_lowercase()) {
                return Some(name);
            }
            if !artist.is_empty() && name.to_lowercase().contains(&artist.to_lowercase()) {
                return Some(name);
            }
        }

        None
    }
}
