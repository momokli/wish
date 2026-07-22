use sqlx::sqlite::SqlitePool;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinSet;

use crate::db;
use crate::deemix::DeemixClient;

pub struct DownloadWorker {
    pool: SqlitePool,
    deemix: DeemixClient,
    output_dir: PathBuf,
    notify: Arc<Notify>,
    ytdlp_available: bool,
    ytdlp_cookies: Option<PathBuf>,
    ytdlp_proxy: Option<String>,
    max_concurrent: usize,
    max_retries: u32,
    download_timeout_secs: u64,
    in_flight: Arc<Mutex<HashSet<i64>>>,
}

impl DownloadWorker {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pool: SqlitePool,
        deemix: DeemixClient,
        output_dir: PathBuf,
        notify: Arc<Notify>,
        ytdlp_available: bool,
        ytdlp_cookies: Option<PathBuf>,
        ytdlp_proxy: Option<String>,
        max_concurrent: usize,
        max_retries: u32,
        download_timeout_secs: u64,
    ) -> Self {
        Self {
            pool,
            deemix,
            output_dir,
            notify,
            ytdlp_available,
            ytdlp_cookies,
            ytdlp_proxy,
            max_concurrent,
            max_retries,
            download_timeout_secs,
            in_flight: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
        }
    }

    pub async fn run(&self) {
        tracing::info!(
            "Download worker: yt-dlp={}, concurrent={}",
            self.ytdlp_available,
            self.max_concurrent
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

        // ── Phase 1: burst-enqueue ALL Spotify URLs to deemix (fire-and-forget) ──
        let mut uuids: HashMap<i64, Option<String>> = HashMap::new();
        let spotify_count = pending.iter().filter(|s| s.source == "spotify").count();
        if spotify_count > 0 {
            tracing::info!("Burst-enqueueing {} Spotify URLs to deemix", spotify_count);
            for sub in &pending {
                if sub.source == "spotify" {
                    match self.deemix.add_to_queue(&sub.spotify_url).await {
                        Ok(uuid) => {
                            uuids.insert(sub.id, uuid.clone());
                            if let Some(ref u) = uuid {
                                tracing::info!("[{}] enqueued to deemix (uuid={})", sub.id, u);
                            } else {
                                tracing::info!(
                                    "[{}] enqueued to deemix (duplicate/already there)",
                                    sub.id
                                );
                            }
                        }
                        Err(e) => {
                            tracing::warn!("[{}] deemix enqueue failed: {e}", sub.id);
                            uuids.insert(sub.id, None);
                        }
                    }
                }
            }
        }

        // ── Phase 2: poll + fallbacks concurrently ──
        let pool = self.pool.clone();
        let deemix = self.deemix.clone();
        let dir = self.output_dir.clone();
        let yt = self.ytdlp_available;
        let cookies = self.ytdlp_cookies.clone();
        let proxy = self.ytdlp_proxy.clone();
        let in_flight = self.in_flight.clone();

        let mut set = JoinSet::new();
        let mut n = 0usize;
        for sub in pending {
            let id = sub.id;
            {
                let mut guard = in_flight.lock().await;
                if !guard.insert(id) {
                    tracing::debug!("[{id}] already in-flight, skipping");
                    continue;
                }
            }

            let f_in_flight = in_flight.clone();
            let f_pool = pool.clone();
            let f_deemix = deemix.clone();
            let f_dir = dir.clone();
            let f_cookies = cookies.clone();
            let f_proxy = proxy.clone();
            let f_max_retries = self.max_retries;
            let f_timeout_secs = self.download_timeout_secs;
            let f_uuid = uuids.remove(&id).flatten();
            set.spawn(async move {
                process_one(
                    f_pool,
                    f_deemix,
                    f_dir,
                    yt,
                    f_cookies,
                    f_proxy,
                    f_max_retries,
                    f_timeout_secs,
                    sub,
                    f_uuid,
                )
                .await;
                f_in_flight.lock().await.remove(&id);
            });
            n += 1;
            if n >= self.max_concurrent {
                set.join_next().await;
                n -= 1;
            }
        }
        while (set.join_next().await).is_some() {}
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
async fn process_one(
    pool: SqlitePool,
    deemix: DeemixClient,
    dir: PathBuf,
    ytdlp: bool,
    ytdlp_cookies: Option<PathBuf>,
    ytdlp_proxy: Option<String>,
    max_retries: u32,
    timeout_secs: u64,
    sub: crate::models::Submission,
    pre_enqueued_uuid: Option<String>,
) {
    let id = sub.id;
    let src = sub.source.as_str();
    let url = &sub.spotify_url;
    tracing::info!("[{id}] {src}: {url}");

    note(&pool, id, "start", &format!("{src} pipeline starting")).await;

    match src {
        "spotify" => {
            // L1: deemix — uses pre-enqueued UUID from burst phase
            note(&pool, id, "deemix", "polling deemix").await;
            if try_deemix(&pool, &deemix, &dir, &sub, timeout_secs, pre_enqueued_uuid)
                .await
                .is_ok()
            {
                return;
            }
            note(&pool, id, "deemix", "deemix failed, falling back to spotDL").await;

            // L2: spotDL
            note(&pool, id, "spotDL", "starting spotDL").await;
            if try_spotdl(&pool, &dir, &sub, max_retries, timeout_secs)
                .await
                .is_ok()
            {
                return;
            }
            note(
                &pool,
                id,
                "spotDL",
                "spotDL exhausted, falling back to yt-dlp",
            )
            .await;

            // L3: yt-dlp
            if ytdlp {
                if let (Some(t), Some(a)) =
                    (sub.track_title.as_deref(), sub.track_artist.as_deref())
                {
                    let q = format!("ytsearch1:{a} - {t}");
                    let tmpl =
                        dir.join("%(artist,uploader|Unknown Artist)s - %(title)s [%(id)s].%(ext)s");
                    note(&pool, id, "yt-dlp", &format!("searching: {q}")).await;
                    if run_ytdlp(
                        &pool,
                        &dir,
                        id,
                        &tmpl,
                        &q,
                        ytdlp_cookies.as_ref(),
                        ytdlp_proxy.as_deref(),
                        max_retries,
                        timeout_secs,
                    )
                    .await
                    .is_ok()
                    {
                        return;
                    }
                    note(&pool, id, "yt-dlp", "yt-dlp exhausted").await;
                }
            }

            fail(&pool, id, "deemix + spotDL + yt-dlp all failed").await;
        }
        "youtube" => {
            let query = if let (Some(t), Some(a)) =
                (sub.track_title.as_deref(), sub.track_artist.as_deref())
            {
                format!("ytsearch1:{a} - {t}")
            } else {
                url.clone()
            };
            let tmpl = dir.join("%(artist,uploader|Unknown Artist)s - %(title)s [%(id)s].%(ext)s");
            note(&pool, id, "yt-dlp", &format!("searching: {query}")).await;
            if let Err(e) = run_ytdlp(
                &pool,
                &dir,
                id,
                &tmpl,
                &query,
                ytdlp_cookies.as_ref(),
                ytdlp_proxy.as_deref(),
                max_retries,
                timeout_secs,
            )
            .await
            {
                fail(&pool, id, &e.to_string()).await;
            }
        }
        "soundcloud" => {
            let tmpl = dir.join("%(artist,uploader|Unknown Artist)s - %(title)s [%(id)s].%(ext)s");
            note(&pool, id, "yt-dlp", "downloading SoundCloud URL directly").await;
            if let Err(e) = run_ytdlp(
                &pool,
                &dir,
                id,
                &tmpl,
                url,
                ytdlp_cookies.as_ref(),
                ytdlp_proxy.as_deref(),
                max_retries,
                timeout_secs,
            )
            .await
            {
                fail(&pool, id, &e.to_string()).await;
            }
        }
        other => fail(&pool, id, &format!("unknown source: {other}")).await,
    }
}

// ── Layers ──

/// Try deemix download. If `pre_enqueued_uuid` is set (from the burst phase),
/// skips add_to_queue and goes straight to polling.
async fn try_deemix(
    pool: &SqlitePool,
    deemix: &DeemixClient,
    dir: &Path,
    sub: &crate::models::Submission,
    timeout_secs: u64,
    pre_enqueued_uuid: Option<String>,
) -> anyhow::Result<()> {
    if sub.filename.is_none() {
        let _ = db::update_submission_status(pool, sub.id, "stage2_deemix", None, None, None).await;
    }

    let poll_uuid = if let Some(uuid) = pre_enqueued_uuid {
        // Already enqueued in the burst phase — go straight to polling
        tracing::info!("[{}] using pre-enqueued deemix uuid={}", sub.id, uuid);
        uuid
    } else {
        // No pre-enqueued UUID — enqueue now (fallback for retries)
        note(pool, sub.id, "deemix", "adding to deemix queue").await;
        let uuid = deemix.add_to_queue(&sub.spotify_url).await?;
        match uuid {
            Some(ref u) => u.clone(),
            None => {
                note(
                    pool,
                    sub.id,
                    "deemix",
                    "no UUID from addToQueue, searching queue for match",
                )
                .await;
                let map = deemix.get_queue_map().await?;
                let found = map.iter().find(|(_, item)| {
                    let title_match = sub
                        .track_title
                        .as_deref()
                        .map(|t| item.title.to_lowercase().contains(&t.to_lowercase()))
                        .unwrap_or(false);
                    let artist_match = sub
                        .track_artist
                        .as_deref()
                        .map(|a| item.artist.to_lowercase().contains(&a.to_lowercase()))
                        .unwrap_or(false);
                    title_match && artist_match
                });
                match found {
                    Some((u, item)) => {
                        tracing::info!(
                            "Found existing deemix queue item: uuid={} title={} status={}",
                            u,
                            item.title,
                            item.status
                        );
                        u.clone()
                    }
                    None => {
                        anyhow::bail!(
                            "deemix: item queued but not found in queue (no UUID, no title match)"
                        );
                    }
                }
            }
        }
    };

    note(
        pool,
        sub.id,
        "deemix",
        &format!("polling uuid={} (timeout {timeout_secs}s)", poll_uuid),
    )
    .await;

    match deemix.poll_by_uuid(&poll_uuid, timeout_secs).await {
        Ok(Some(item))
            if item.status == "finished"
                || item.status == "downloaded"
                || item.status == "completed" =>
        {
            if let Some(f) = item.files.first() {
                let filename = &f.filename;
                let full_path = dir.join(filename);
                if full_path.exists() {
                    tracing::info!("Deemix file found on disk: {} (from response)", filename);
                    return done(pool, dir, sub.id, filename, "deemix").await;
                }
                tracing::warn!(
                    "Deemix reported file {} but not found at {} — trying scan",
                    filename,
                    full_path.display()
                );
            }
            if let Some(f) = scan_recent(dir, 30).await {
                return done(pool, dir, sub.id, &f, "deemix").await;
            }
            anyhow::bail!("deemix finished but file not found on disk");
        }
        Ok(Some(item)) => {
            let msg = format!("deemix ended with status: {}", item.status);
            anyhow::bail!(msg);
        }
        Ok(None) => anyhow::bail!("deemix: track vanished from queue before completion"),
        Err(e) => anyhow::bail!("deemix poll error: {e}"),
    }
}

async fn try_spotdl(
    pool: &SqlitePool,
    dir: &Path,
    sub: &crate::models::Submission,
    max_retries: u32,
    timeout_secs: u64,
) -> anyhow::Result<()> {
    if sub.filename.is_none() {
        let _ = db::update_submission_status(pool, sub.id, "stage3_spotdl", None, None, None).await;
    }
    let fmt = dir
        .join("{title} - {artists}.{output-ext}")
        .to_string_lossy()
        .to_string();
    for a in 1..=max_retries {
        note(
            pool,
            sub.id,
            "spotDL",
            &format!("attempt {a}/{max_retries}"),
        )
        .await;
        tracing::info!("[{}] spotDL {a}/{max_retries}", sub.id);
        let fut = tokio::process::Command::new("spotdl")
            .args([
                "download",
                &sub.spotify_url,
                "--output",
                &fmt,
                "--bitrate",
                "320k",
                "--overwrite",
                "skip",
            ])
            .output();
        let o = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), fut)
            .await
            .map_err(|_| anyhow::anyhow!("spotDL timed out after {timeout_secs}s"))??;
        if o.status.success() {
            if let Some(f) = scan_recent(dir, 5).await {
                return done(pool, dir, sub.id, &f, "spotDL").await;
            }
            note(
                pool,
                sub.id,
                "spotDL",
                "spotDL exited OK but no output file found",
            )
            .await;
            tracing::warn!("[{}] spotDL OK but no output file", sub.id);
        } else {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let reason = stderr.lines().last().unwrap_or("");
            note(
                pool,
                sub.id,
                "spotDL",
                &format!("attempt {a} failed: {reason}"),
            )
            .await;
            tracing::warn!("[{}] spotDL {a} failed: {reason}", sub.id);
        }
        if a < max_retries {
            tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(a - 1))).await;
        }
    }
    anyhow::bail!("spotDL failed after {max_retries} attempts");
}

#[allow(clippy::too_many_arguments)]
async fn run_ytdlp(
    pool: &SqlitePool,
    dir: &Path,
    id: i64,
    tmpl: &Path,
    url: &str,
    cookies: Option<&PathBuf>,
    proxy: Option<&str>,
    max_retries: u32,
    timeout_secs: u64,
) -> anyhow::Result<()> {
    let t = tmpl.to_string_lossy().to_string();
    for a in 1..=max_retries {
        note(pool, id, "yt-dlp", &format!("attempt {a}/{max_retries}")).await;
        tracing::info!("[{id}] yt-dlp {a}/{max_retries}");
        let mut cmd = tokio::process::Command::new("yt-dlp");
        cmd.args([
            "-x",
            "--audio-format",
            "mp3",
            "--audio-quality",
            "0",
            "--embed-metadata",
            "--embed-thumbnail",
            "--no-playlist",
            "--no-overwrites",
        ]);
        if let Some(c) = cookies {
            cmd.arg("--cookies").arg(c);
        }
        if let Some(p) = proxy {
            cmd.arg("--proxy").arg(p);
        }
        cmd.args(["--print", "after_move:filepath", "-o", &t, url]);
        let fut = cmd.output();
        let o = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), fut)
            .await
            .map_err(|_| anyhow::anyhow!("yt-dlp timed out after {timeout_secs}s"))??;
        if o.status.success() {
            let out = String::from_utf8_lossy(&o.stdout);
            if let Some(fp) = out.lines().last().filter(|l| !l.trim().is_empty()) {
                let name = Path::new(fp)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| fp.to_string());

                // Persist artist/title from filename if not already set
                let stem = name.rsplitn(2, '.').nth(1).unwrap_or(&name);
                let (artist_opt, title) = parse_stem_title(stem);
                let _ = db::update_track_metadata(pool, id, &title, artist_opt.as_deref()).await;

                return done(pool, dir, id, &name, "yt-dlp").await;
            }
            anyhow::bail!("yt-dlp succeeded but no filepath printed to stdout");
        }
        let stderr = String::from_utf8_lossy(&o.stderr);
        let rsn = reason(&stderr);
        note(pool, id, "yt-dlp", &format!("attempt {a} failed: {rsn}")).await;
        if a < max_retries {
            let delay = std::time::Duration::from_secs(2u64.pow(a - 1));
            tracing::warn!("[{id}] yt-dlp {a} failed ({delay:?}): {rsn}");
            tokio::time::sleep(delay).await;
        } else {
            anyhow::bail!("{rsn}");
        }
    }
    unreachable!()
}

/// Parse "Artist - Title" or "Unknown Artist - Title" from a yt-dlp filename stem.
/// Returns (artist_opt, title).
fn parse_stem_title(stem: &str) -> (Option<String>, String) {
    // Strip trailing " [id]" if present
    let without_id = match stem.rsplitn(2, " [").next() {
        Some(s) => s,
        None => return (None, stem.to_string()),
    };
    match without_id.split_once(" - ") {
        Some((artist, title)) => {
            let artist = match artist {
                "NA" | "Unknown Artist" => None,
                a => Some(a.to_string()),
            };
            (artist, title.to_string())
        }
        None => (None, without_id.to_string()),
    }
}

// ── Helpers ──

async fn note(pool: &SqlitePool, id: i64, layer: &str, msg: &str) {
    let _ = db::append_attempt(pool, id, layer, false, None, None, None, Some(msg)).await;
}

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
    let container = name.split('.').last().map(|e| e.to_lowercase());
    let note = format!("downloaded via {stage}");
    db::update_submission_status(pool, id, "ready", Some(name), sz, Some(&note)).await?;
    let _ = sqlx::query(
        "UPDATE submissions SET first_available_at = COALESCE(first_available_at, unixepoch()) WHERE id = ?",
    )
    .bind(id)
    .execute(pool)
    .await;
    let _ = db::append_attempt(
        pool,
        id,
        stage,
        true,
        Some(name),
        None,
        container.as_deref(),
        None,
    )
    .await;
    tracing::info!("[{id}] ready [{stage}] {name}",);
    Ok(())
}

async fn fail(pool: &SqlitePool, id: i64, msg: &str) {
    let _ = db::update_submission_status(pool, id, "failed", None, None, Some(msg)).await;
    let _ = db::append_attempt(pool, id, "fail", false, None, None, None, Some(msg)).await;
    tracing::error!("[{id}] FAILED: {msg}");
}

async fn scan_recent(dir: &Path, within_secs: u64) -> Option<String> {
    use std::collections::VecDeque;
    let deadline =
        std::time::SystemTime::now().checked_sub(std::time::Duration::from_secs(within_secs))?;
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
                    if mt >= deadline {
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
    }
    best.map(|(n, _)| n)
}
