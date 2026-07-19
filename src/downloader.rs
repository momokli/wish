use sqlx::sqlite::SqlitePool;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Notify;
use tokio::task::JoinSet;

use crate::db;
use crate::deemix::DeemixClient;

const MAX_CONCURRENT: usize = 3;
const MAX_RETRIES: u32 = 3;

pub struct DownloadWorker {
    pool: SqlitePool,
    deemix: DeemixClient,
    output_dir: PathBuf,
    notify: Arc<Notify>,
    ytdlp_available: bool,
}

impl DownloadWorker {
    pub fn new(
        pool: SqlitePool,
        deemix: DeemixClient,
        output_dir: PathBuf,
        notify: Arc<Notify>,
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

    pub async fn run(self) {
        tracing::info!(
            "Download worker started (yt-dlp: {}, max concurrent: {})",
            self.ytdlp_available,
            MAX_CONCURRENT
        );

        loop {
            tokio::select! {
                _ = self.notify.notified() => {}
                _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {}
            }
            if let Err(e) = self.process_pending().await {
                tracing::error!("Error processing: {}", e);
            }
        }
    }

    async fn process_pending(&self) -> anyhow::Result<()> {
        let pending = db::get_pending_submissions(&self.pool).await?;
        if pending.is_empty() {
            return Ok(());
        }

        tracing::info!("Processing {} pending submission(s)", pending.len());

        let pool = self.pool.clone();
        let deemix = self.deemix.clone();
        let output_dir = self.output_dir.clone();
        let ytdlp = self.ytdlp_available;

        let mut set = JoinSet::new();
        let mut inflight = 0usize;

        for sub in pending {
            let p = pool.clone();
            let d = deemix.clone();
            let o = output_dir.clone();

            set.spawn(async move { process_one(p, d, o, ytdlp, sub).await });
            inflight += 1;

            if inflight >= MAX_CONCURRENT {
                set.join_next().await;
                inflight -= 1;
            }
        }

        while (set.join_next().await).is_some() {}
        Ok(())
    }
}

async fn process_one(
    pool: SqlitePool,
    deemix: DeemixClient,
    output_dir: PathBuf,
    ytdlp_available: bool,
    submission: crate::models::Submission,
) {
    let source = submission.source.as_str();
    tracing::info!(
        "[{}] processing {} [{}]",
        submission.id,
        submission.spotify_url,
        source
    );

    let result = match source {
        "youtube" | "soundcloud" => {
            download_yt(pool.clone(), &output_dir, ytdlp_available, &submission).await
        }
        _ => {
            download_spotify(
                pool.clone(),
                &deemix,
                &output_dir,
                ytdlp_available,
                &submission,
            )
            .await
        }
    };

    if let Err(e) = result {
        tracing::error!("[{}] permanently failed: {}", submission.id, e);
        let _ = db::update_submission_status(
            &pool,
            submission.id,
            "failed",
            None,
            None,
            Some(&format!("All stages exhausted: {}", e)),
        )
        .await;
    }
}

// ── yt-dlp: YouTube / SoundCloud / Spotify fallback ──

async fn download_yt(
    pool: SqlitePool,
    output_dir: &Path,
    ytdlp_available: bool,
    sub: &crate::models::Submission,
) -> anyhow::Result<()> {
    if !ytdlp_available {
        anyhow::bail!("yt-dlp not available");
    }
    let template = output_dir.join("%(artist)s - %(title)s [%(id)s].%(ext)s");
    run_ytdlp(
        &pool,
        output_dir,
        sub.id,
        &template,
        &sub.spotify_url,
        "yt-dlp",
    )
    .await
}

async fn run_ytdlp(
    pool: &SqlitePool,
    output_dir: &Path,
    submission_id: i64,
    template: &Path,
    url_or_query: &str,
    label: &str,
) -> anyhow::Result<()> {
    let tmpl = template.to_string_lossy().to_string();

    for attempt in 1..=MAX_RETRIES {
        tracing::info!(
            "[{}] {} attempt {}/{}",
            submission_id,
            label,
            attempt,
            MAX_RETRIES
        );

        let out = tokio::process::Command::new("yt-dlp")
            .args([
                "-f",
                "bestaudio",
                "--extract-audio",
                "--audio-format",
                "mp3",
                "--audio-quality",
                "0",
                "--embed-metadata",
                "--embed-thumbnail",
                "--no-playlist",
                "--no-overwrites",
                "--print",
                "after_move:filepath",
                "-o",
                &tmpl,
                url_or_query,
            ])
            .output()
            .await;

        match out {
            Ok(o) if o.status.success() => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                if let Some(filepath) = stdout.lines().last().filter(|l| !l.trim().is_empty()) {
                    let filename = Path::new(filepath)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| filepath.to_string());
                    return mark_ready(pool, output_dir, submission_id, &filename, label).await;
                }
                // fallback: newest mp3
                if let Some(f) = find_newest_mp3(output_dir).await {
                    return mark_ready(pool, output_dir, submission_id, &f, label).await;
                }
                tracing::warn!("[{}] {}: can't determine output file", submission_id, label);
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                let tail: String = stderr.lines().rev().take(2).collect::<Vec<_>>().join(" | ");
                tracing::warn!(
                    "[{}] {} failed (attempt {}): {}",
                    submission_id,
                    label,
                    attempt,
                    tail
                );
            }
            Err(e) => {
                tracing::warn!(
                    "[{}] {} error (attempt {}): {}",
                    submission_id,
                    label,
                    attempt,
                    e
                );
            }
        }

        if attempt < MAX_RETRIES {
            tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(attempt - 1))).await;
        }
    }

    anyhow::bail!("{} failed after {} attempts", label, MAX_RETRIES);
}

// ── Spotify pipeline ──

async fn download_spotify(
    pool: SqlitePool,
    deemix: &DeemixClient,
    output_dir: &Path,
    ytdlp_available: bool,
    sub: &crate::models::Submission,
) -> anyhow::Result<()> {
    // Stage 1: deemix
    let _ = db::update_submission_status(&pool, sub.id, "stage2_deemix", None, None, None).await;
    if try_deemix(&pool, deemix, output_dir, sub).await {
        return Ok(());
    }

    // Stage 2: spotDL
    let _ = db::update_submission_status(&pool, sub.id, "stage3_spotdl", None, None, None).await;
    if try_spotdl(&pool, output_dir, sub).await {
        return Ok(());
    }

    // Stage 3: yt-dlp search fallback
    if ytdlp_available {
        if let (Some(title), Some(artist)) = (&sub.track_title, &sub.track_artist) {
            let q = format!("ytsearch1:{} - {}", artist, title);
            let template = output_dir.join("%(artist)s - %(title)s [%(id)s].%(ext)s");
            if run_ytdlp(
                &pool,
                output_dir,
                sub.id,
                &template,
                &q,
                "yt-dlp (spotify fallback)",
            )
            .await
            .is_ok()
            {
                return Ok(());
            }
        }
    }

    anyhow::bail!("deemix + spotDL + yt-dlp all failed");
}

async fn try_deemix(
    pool: &SqlitePool,
    deemix: &DeemixClient,
    output_dir: &Path,
    sub: &crate::models::Submission,
) -> bool {
    tracing::info!("[{}] trying deemix", sub.id);

    match deemix.add_to_queue(&sub.spotify_url).await {
        Ok(()) => match deemix.poll_until_done(&sub.spotify_url, 300).await {
            Ok(Some(item)) if item.status == "finished" || item.status == "downloaded" => {
                if let Some(f) = find_newest_mp3(output_dir).await {
                    let _ = mark_ready(pool, output_dir, sub.id, &f, "deemix").await;
                    return true;
                }
                tracing::warn!("[{}] deemix done but no file found", sub.id);
            }
            Ok(Some(item)) => tracing::warn!("[{}] deemix status: {}", sub.id, item.status),
            Ok(None) => tracing::warn!("[{}] deemix item vanished", sub.id),
            Err(e) => tracing::warn!("[{}] deemix poll error: {}", sub.id, e),
        },
        Err(e) => tracing::warn!("[{}] deemix queue error: {}", sub.id, e),
    }
    false
}

async fn try_spotdl(pool: &SqlitePool, output_dir: &Path, sub: &crate::models::Submission) -> bool {
    let fmt = output_dir.join("{title} - {artists}.{ext}");
    let fmt_str = fmt.to_string_lossy().to_string();

    for attempt in 1..=MAX_RETRIES {
        tracing::info!("[{}] spotDL attempt {}/{}", sub.id, attempt, MAX_RETRIES);

        let out = tokio::process::Command::new("spotdl")
            .args([
                "download",
                &sub.spotify_url,
                "--output",
                &fmt_str,
                "--bitrate",
                "320k",
                "--no-overwrites",
            ])
            .output()
            .await;

        match out {
            Ok(o) if o.status.success() => {
                if let Some(f) = find_newest_mp3(output_dir).await {
                    let _ = mark_ready(pool, output_dir, sub.id, &f, "spotDL").await;
                    return true;
                }
                tracing::warn!("[{}] spotDL done but no file found", sub.id);
                return false;
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                tracing::warn!(
                    "[{}] spotDL failed (attempt {}): {}",
                    sub.id,
                    attempt,
                    stderr.lines().last().unwrap_or("")
                );
            }
            Err(e) => tracing::warn!("[{}] spotDL error (attempt {}): {}", sub.id, attempt, e),
        }

        if attempt < MAX_RETRIES {
            tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(attempt - 1))).await;
        }
    }
    false
}

// ── Helpers ──

async fn mark_ready(
    pool: &SqlitePool,
    output_dir: &Path,
    id: i64,
    filename: &str,
    stage: &str,
) -> anyhow::Result<()> {
    let size = tokio::fs::metadata(output_dir.join(filename))
        .await
        .ok()
        .map(|m| m.len() as i64);

    db::update_submission_status(pool, id, "ready", Some(filename), size, None).await?;
    tracing::info!("[{}] ready via {}: {}", id, stage, filename);
    Ok(())
}

async fn find_newest_mp3(dir: &Path) -> Option<String> {
    let mut entries = tokio::fs::read_dir(dir).await.ok()?;
    let mut best: Option<(String, std::time::SystemTime)> = None;

    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".mp3") {
            continue;
        }
        if let Ok(meta) = entry.metadata().await {
            if let Ok(mt) = meta.modified() {
                match &best {
                    None => best = Some((name, mt)),
                    Some((_, prev)) if mt > *prev => best = Some((name, mt)),
                    _ => {}
                }
            }
        }
    }
    best.map(|(n, _)| n)
}
