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
            "Download worker: yt-dlp={}, concurrent={}",
            self.ytdlp_available,
            MAX_CONCURRENT
        );
        loop {
            tokio::select! {
                _ = self.notify.notified() => {}
                _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {}
            }
            if let Err(e) = self.process_pending().await {
                tracing::error!("Error: {e}");
            }
        }
    }

    async fn process_pending(&self) -> anyhow::Result<()> {
        let pending = db::get_pending_submissions(&self.pool).await?;
        if pending.is_empty() {
            return Ok(());
        }
        tracing::info!("Processing {} pending", pending.len());

        let pool = self.pool.clone();
        let deemix = self.deemix.clone();
        let dir = self.output_dir.clone();
        let yt = self.ytdlp_available;

        let mut set = JoinSet::new();
        let mut n = 0usize;
        for sub in pending {
            set.spawn(process_one(
                pool.clone(),
                deemix.clone(),
                dir.clone(),
                yt,
                sub,
            ));
            n += 1;
            if n >= MAX_CONCURRENT {
                set.join_next().await;
                n -= 1;
            }
        }
        while (set.join_next().await).is_some() {}
        Ok(())
    }
}

async fn process_one(
    pool: SqlitePool,
    deemix: DeemixClient,
    dir: PathBuf,
    ytdlp: bool,
    sub: crate::models::Submission,
) {
    let id = sub.id;
    let src = sub.source.as_str();
    let url = &sub.spotify_url;
    tracing::info!("[{id}] {src}: {url}");

    match src {
        "spotify" => {
            // L1: deemix → L2: spotDL → L3: yt-dlp
            if try_deemix(&pool, &deemix, &dir, &sub).await.is_ok() {
                return;
            }
            if try_spotdl(&pool, &dir, &sub).await.is_ok() {
                return;
            }
            if ytdlp {
                if let (Some(t), Some(a)) =
                    (sub.track_title.as_deref(), sub.track_artist.as_deref())
                {
                    let q = format!("ytsearch1:{a} - {t}");
                    let tmpl = dir.join("%(artist)s - %(title)s [%(id)s].%(ext)s");
                    if let Err(e) = run_ytdlp(&pool, &dir, id, &tmpl, &q).await {
                        fail(&pool, id, &e.to_string()).await;
                    }
                } else {
                    fail(&pool, id, "no metadata for search").await;
                }
            } else {
                fail(&pool, id, "yt-dlp not available").await;
            }
        }
        "youtube" => {
            // Use ytsearch1: with metadata — avoids YouTube bot detection on direct URLs
            let query = if let (Some(t), Some(a)) =
                (sub.track_title.as_deref(), sub.track_artist.as_deref())
            {
                format!("ytsearch1:{a} - {t}")
            } else {
                url.clone()
            };
            let tmpl = dir.join("%(artist)s - %(title)s [%(id)s].%(ext)s");
            if let Err(e) = run_ytdlp(&pool, &dir, id, &tmpl, &query).await {
                fail(&pool, id, &e.to_string()).await;
            }
        }
        "soundcloud" => {
            // Direct URL — SoundCloud doesn't need ytsearch1:
            let tmpl = dir.join("%(artist)s - %(title)s [%(id)s].%(ext)s");
            if let Err(e) = run_ytdlp(&pool, &dir, id, &tmpl, url).await {
                fail(&pool, id, &e.to_string()).await;
            }
        }
        other => fail(&pool, id, &format!("unknown: {other}")).await,
    }
}

// ── Layers ──

async fn try_deemix(
    pool: &SqlitePool,
    deemix: &DeemixClient,
    dir: &Path,
    sub: &crate::models::Submission,
) -> anyhow::Result<()> {
    let _ = db::update_submission_status(pool, sub.id, "stage2_deemix", None, None, None).await;
    deemix.add_to_queue(&sub.spotify_url).await?;
    match deemix.poll_until_done(&sub.spotify_url, 300).await {
        Ok(Some(item))
            if item.status == "finished"
                || item.status == "downloaded"
                || item.status == "completed" =>
        {
            if let Some(f) = newest(dir).await {
                return done(pool, dir, sub.id, &f, "deemix").await;
            }
            anyhow::bail!("file not found");
        }
        Ok(Some(item)) => anyhow::bail!("status: {}", item.status),
        Ok(None) => anyhow::bail!("vanished"),
        Err(e) => anyhow::bail!("{e}"),
    }
}

async fn try_spotdl(
    pool: &SqlitePool,
    dir: &Path,
    sub: &crate::models::Submission,
) -> anyhow::Result<()> {
    let _ = db::update_submission_status(pool, sub.id, "stage3_spotdl", None, None, None).await;
    let fmt = dir
        .join("{title} - {artists}.{ext}")
        .to_string_lossy()
        .to_string();
    for a in 1..=MAX_RETRIES {
        let o = tokio::process::Command::new("spotdl")
            .args([
                "download",
                &sub.spotify_url,
                "--output",
                &fmt,
                "--bitrate",
                "320k",
                "--no-overwrites",
            ])
            .output()
            .await?;
        if o.status.success() {
            if let Some(f) = newest(dir).await {
                return done(pool, dir, sub.id, &f, "spotDL").await;
            }
            anyhow::bail!("no output");
        }
        if a < MAX_RETRIES {
            tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(a - 1))).await;
        }
    }
    anyhow::bail!("spotDL failed");
}

async fn run_ytdlp(
    pool: &SqlitePool,
    dir: &Path,
    id: i64,
    tmpl: &Path,
    url: &str,
) -> anyhow::Result<()> {
    let t = tmpl.to_string_lossy().to_string();
    for a in 1..=MAX_RETRIES {
        tracing::info!("[{id}] yt-dlp {a}/{MAX_RETRIES}");
        let o = tokio::process::Command::new("yt-dlp")
            .args([
                "-x",
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
                &t,
                url,
            ])
            .output()
            .await?;
        if o.status.success() {
            let out = String::from_utf8_lossy(&o.stdout);
            if let Some(fp) = out.lines().last().filter(|l| !l.trim().is_empty()) {
                let name = Path::new(fp)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| fp.to_string());
                return done(pool, dir, id, &name, "yt-dlp").await;
            }
            if let Some(f) = newest(dir).await {
                return done(pool, dir, id, &f, "yt-dlp").await;
            }
            anyhow::bail!("can't find output");
        }
        let stderr = String::from_utf8_lossy(&o.stderr);
        let reason = reason(&stderr);
        if a < MAX_RETRIES {
            let d = std::time::Duration::from_secs(2u64.pow(a - 1));
            tracing::warn!("[{id}] yt-dlp {a} failed: {reason} ({d:?})");
            tokio::time::sleep(d).await;
        } else {
            anyhow::bail!("{reason}");
        }
    }
    unreachable!()
}

// ── Helpers ──

fn reason(s: &str) -> String {
    let lo = s.to_lowercase();
    if lo.contains("sign in") || lo.contains("bot") {
        "YouTube blocks this request".into()
    } else if lo.contains("drm") {
        "DRM protected".into()
    } else if lo.contains("404") || lo.contains("not found") {
        "Not found".into()
    } else if lo.contains("private") {
        "Private".into()
    } else {
        s.lines()
            .filter(|l| !l.trim().is_empty())
            .last()
            .unwrap_or("unknown")
            .trim()
            .to_string()
    }
}

async fn done(
    pool: &SqlitePool,
    dir: &Path,
    id: i64,
    name: &str,
    stage: &str,
) -> anyhow::Result<()> {
    let sz = tokio::fs::metadata(dir.join(name))
        .await
        .ok()
        .map(|m| m.len() as i64);
    let note = format!("downloaded via {stage}");
    db::update_submission_status(pool, id, "ready", Some(name), sz, Some(&note)).await?;
    tracing::info!("[{id}] ready [{stage}] {name}");
    Ok(())
}

async fn fail(pool: &SqlitePool, id: i64, reason: &str) {
    let _ = db::update_submission_status(pool, id, "failed", None, None, Some(reason)).await;
    tracing::error!("[{id}] FAILED: {reason}");
}

async fn newest(dir: &Path) -> Option<String> {
    use std::collections::VecDeque;
    let mut best: Option<(String, std::time::SystemTime)> = None;
    let mut dirs = VecDeque::new();
    dirs.push_back(dir.to_path_buf());

    while let Some(d) = dirs.pop_front() {
        let mut entries = tokio::fs::read_dir(&d).await.ok()?;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let ft = entry.file_type().await.ok()?;
            if ft.is_dir() {
                dirs.push_back(entry.path());
                continue;
            }
            let n = entry.file_name().to_string_lossy().to_string();
            if !n.ends_with(".mp3") && !n.ends_with(".flac") && !n.ends_with(".m4a") {
                continue;
            }
            if let Ok(meta) = entry.metadata().await {
                if let Ok(mt) = meta.modified() {
                    // Store path relative to output dir so done() can find it
                    if let Ok(rel) = entry.path().strip_prefix(dir) {
                        let rel_str = rel.to_string_lossy().to_string();
                        if best.as_ref().map_or(true, |(_, p)| mt > *p) {
                            best = Some((rel_str, mt));
                        }
                    }
                }
            }
        }
    }
    best.map(|(n, _)| n)
}
