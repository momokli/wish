use anyhow::Context;
use sqlx::sqlite::SqlitePool;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinSet;

use crate::db;
use crate::deemix::DeemixClient;

pub struct DownloadWorker {
    pool: SqlitePool,
    deemix: DeemixClient,
    deemix_dir: PathBuf,
    spotdl_dir: PathBuf,
    best_dir: PathBuf,
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
        deemix_dir: PathBuf,
        spotdl_dir: PathBuf,
        best_dir: PathBuf,
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
            deemix_dir,
            spotdl_dir,
            best_dir,
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

        let pool = self.pool.clone();
        let deemix = self.deemix.clone();
        let deemix_dir = self.deemix_dir.clone();
        let spotdl_dir = self.spotdl_dir.clone();
        let best_dir = self.best_dir.clone();
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
            let f_deemix_dir = deemix_dir.clone();
            let f_spotdl_dir = spotdl_dir.clone();
            let f_best_dir = best_dir.clone();
            let f_cookies = cookies.clone();
            let f_proxy = proxy.clone();
            let f_max_retries = self.max_retries;
            let f_timeout_secs = self.download_timeout_secs;
            set.spawn(async move {
                process_one(
                    f_pool,
                    f_deemix,
                    f_deemix_dir,
                    f_spotdl_dir,
                    f_best_dir,
                    yt,
                    f_cookies,
                    f_proxy,
                    f_max_retries,
                    f_timeout_secs,
                    sub,
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
    deemix_dir: PathBuf,
    spotdl_dir: PathBuf,
    best_dir: PathBuf,
    ytdlp: bool,
    ytdlp_cookies: Option<PathBuf>,
    ytdlp_proxy: Option<String>,
    max_retries: u32,
    timeout_secs: u64,
    sub: crate::models::Submission,
) {
    let id = sub.id;
    let src = sub.source.as_str();
    let url = &sub.spotify_url;
    tracing::info!("[{id}] {src}: {url}");

    note(&pool, id, "start", &format!("{src} pipeline starting")).await;

    match src {
        "spotify" => {
            // L1: deemix
            note(&pool, id, "deemix", "polling deemix").await;
            match try_deemix(&pool, &deemix, &deemix_dir, &best_dir, &sub, timeout_secs).await {
                Ok(()) => return,
                Err(e) => {
                    let reason = format!("{e:#}");
                    tracing::warn!("[{id}] L1 deemix failed: {reason}");
                    note(&pool, id, "deemix", &format!("failed: {reason}")).await;
                }
            }

            // L2: spotDL
            note(&pool, id, "spotDL", "starting spotDL").await;
            match try_spotdl(
                &pool,
                &spotdl_dir,
                &best_dir,
                &sub,
                max_retries,
                timeout_secs,
            )
            .await
            {
                Ok(()) => return,
                Err(e) => {
                    let reason = format!("{e:#}");
                    tracing::warn!("[{id}] L2 spotDL failed: {reason}");
                    note(&pool, id, "spotDL", &format!("failed: {reason}")).await;
                }
            }

            // L3: yt-dlp
            if ytdlp {
                if let (Some(t), Some(a)) =
                    (sub.track_title.as_deref(), sub.track_artist.as_deref())
                {
                    let q = format!("ytsearch1:{a} - {t}");
                    let tmpl = spotdl_dir
                        .join("%(artist,uploader|Unknown Artist)s - %(title)s [%(id)s].%(ext)s");
                    note(&pool, id, "yt-dlp", &format!("searching: {q}")).await;
                    match run_ytdlp(
                        &pool,
                        &spotdl_dir,
                        &best_dir,
                        id,
                        &tmpl,
                        &q,
                        ytdlp_cookies.as_ref(),
                        ytdlp_proxy.as_deref(),
                        max_retries,
                        timeout_secs,
                    )
                    .await
                    {
                        Ok(()) => return,
                        Err(e) => {
                            let reason = format!("{e:#}");
                            tracing::warn!("[{id}] L3 yt-dlp failed: {reason}");
                            note(&pool, id, "yt-dlp", &format!("failed: {reason}")).await;
                        }
                    }
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
            let tmpl =
                spotdl_dir.join("%(artist,uploader|Unknown Artist)s - %(title)s [%(id)s].%(ext)s");
            note(&pool, id, "yt-dlp", &format!("searching: {query}")).await;
            if let Err(e) = run_ytdlp(
                &pool,
                &spotdl_dir,
                &best_dir,
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
            let tmpl =
                spotdl_dir.join("%(artist,uploader|Unknown Artist)s - %(title)s [%(id)s].%(ext)s");
            note(&pool, id, "yt-dlp", "downloading SoundCloud URL directly").await;
            if let Err(e) = run_ytdlp(
                &pool,
                &spotdl_dir,
                &best_dir,
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

/// Try deemix L1. Fire-and-forget add_to_queue, then scan the filesystem
/// for a new .mp3 whose ISRC matches the submission. No UUID polling needed.
async fn try_deemix(
    pool: &SqlitePool,
    deemix: &DeemixClient,
    dir: &Path,
    best_dir: &Path,
    sub: &crate::models::Submission,
    timeout_secs: u64,
) -> anyhow::Result<()> {
    if sub.filename.is_none() {
        let _ = db::update_submission_status(pool, sub.id, "stage2_deemix", None, None, None).await;
    }

    note(pool, sub.id, "deemix", "add_to_queue").await;

    // Snapshot existing files before we trigger deemix
    let known = list_files(dir).await;

    // Fire & forget — we don't care about UUID, we scan filesystem instead
    let enqueue = deemix.add_to_queue(&sub.spotify_url).await;
    match &enqueue {
        Ok(Some(e)) => tracing::info!(
            "[{}] deemix enqueued (uuid={}, deezer_id={:?})",
            sub.id,
            e.uuid,
            e.deezer_track_id
        ),
        Ok(None) => tracing::info!(
            "[{}] deemix enqueued (no UUID — scanning filesystem)",
            sub.id
        ),
        Err(e) => tracing::warn!("[{}] deemix add_to_queue error: {e:#}", sub.id),
    }

    // Get expected ISRC for matching
    let sub_isrc: Option<String> = sqlx::query_scalar("SELECT isrc FROM submissions WHERE id = ?")
        .bind(sub.id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();

    // Poll filesystem for matching file
    tracing::info!(
        "[{}] deemix scanning for file (isrc={:?}, timeout={}s)",
        sub.id,
        sub_isrc,
        timeout_secs
    );
    note(
        pool,
        sub.id,
        "deemix",
        &format!("scanning filesystem (isrc={:?})", sub_isrc),
    )
    .await;
    let start = std::time::Instant::now();
    let poll = std::time::Duration::from_secs(2);

    while start.elapsed().as_secs() < timeout_secs {
        let current = list_files(dir).await;
        for f in current.difference(&known) {
            if !f.ends_with(".mp3") && !f.ends_with(".flac") && !f.ends_with(".m4a") {
                continue;
            }
            let full = dir.join(f);
            // If we have an ISRC, verify it matches
            if let Some(ref expected) = sub_isrc {
                if let Ok(Some(file_isrc)) = extract_metadata_isrc(&full).await {
                    if file_isrc.eq_ignore_ascii_case(expected) {
                        tracing::info!(
                            "[{}] deemix file matched by ISRC: {} ({})",
                            sub.id,
                            f,
                            file_isrc
                        );
                        return done(pool, dir, best_dir, sub.id, f, "deemix").await;
                    }
                    tracing::debug!(
                        "[{}] deemix new file ISRC mismatch: {} (file={}, expected={})",
                        sub.id,
                        f,
                        file_isrc,
                        expected
                    );
                }
            } else {
                // No ISRC to compare — accept any new audio file
                return done(pool, dir, best_dir, sub.id, f, "deemix").await;
            }
        }
        tokio::time::sleep(poll).await;
    }

    let elapsed = start.elapsed().as_secs();
    tracing::warn!(
        "[{}] deemix gave up after {elapsed}s (no file with matching ISRC found)",
        sub.id
    );
    note(
        pool,
        sub.id,
        "deemix",
        &format!("timeout after {elapsed}s — no matching file"),
    )
    .await;
    anyhow::bail!("deemix: no matching file found within {elapsed}s");
}

/// Quick ISRC extraction — returns just the ISRC string.
async fn extract_metadata_isrc(path: &Path) -> anyhow::Result<Option<String>> {
    let output = tokio::process::Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-show_entries",
            "format_tags=TSRC",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .await?;
    if output.status.success() {
        let tsrc = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !tsrc.is_empty() {
            return Ok(Some(tsrc));
        }
    }
    Ok(None)
}

/// List all filenames in a directory (flat, non-recursive).
async fn list_files(dir: &Path) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            set.insert(entry.file_name().to_string_lossy().to_string());
        }
    }
    set
}

async fn try_spotdl(
    pool: &SqlitePool,
    dir: &Path,
    best_dir: &Path,
    sub: &crate::models::Submission,
    max_retries: u32,
    timeout_secs: u64,
) -> anyhow::Result<()> {
    if sub.filename.is_none() {
        let _ = db::update_submission_status(pool, sub.id, "stage3_spotdl", None, None, None).await;
    }
    let fmt = dir
        .join(format!(
            "__w{id}__{{title}} - {{artists}}.{{output-ext}}",
            id = sub.id
        ))
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
        let spotify_url = crate::spotify::spotify_uri_to_url(&sub.spotify_url);
        tracing::info!("[{}] spotDL {a}/{max_retries}", sub.id);
        let fut = tokio::process::Command::new("spotdl")
            .args([
                "download",
                &spotify_url,
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
        let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
        if o.status.success() {
            // Small delay — spotDL may still be flushing the file to disk
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            // Find the file by the unique prefix we gave it
            let prefix = format!("__w{}__", sub.id);
            if let Some(f) = find_by_prefix(dir, &prefix).await {
                tracing::info!("[{}] spotDL downloaded: {}", sub.id, f);
                return done(pool, dir, best_dir, sub.id, &f, "spotDL").await;
            }
            // Fallback: stdout parsing + scan
            if let Some(name) = stdout
                .lines()
                .find(|l| l.trim().starts_with("Downloaded"))
                .and_then(|l| l.split('"').nth(1))
            {
                let expected = format!("{name}.mp3");
                let full_path = dir.join(&expected);
                if full_path.exists() {
                    return done(pool, dir, best_dir, sub.id, &expected, "spotDL").await;
                }
            }
            if let Some(f) = scan_recent(dir, 15).await {
                return done(pool, dir, best_dir, sub.id, &f, "spotDL").await;
            }
            // ── File not found — log diagnostics ──
            tracing::warn!(
                "[{}] spotDL OK but no output file\n  spotDL stdout: {}\n  spotDL stderr: {}",
                sub.id,
                stdout,
                stderr
            );
            // List output dir for debugging
            if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
                let mut listing = String::new();
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let meta = entry.metadata().await;
                    let sz = meta.as_ref().ok().map(|m| m.len());
                    let modified = meta
                        .as_ref()
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .map(|t| {
                            let d = t
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                            format!("{d}")
                        })
                        .unwrap_or_default();
                    listing.push_str(&format!(
                        "  {} ({} bytes, mtime={})\n",
                        name,
                        sz.unwrap_or(0),
                        modified
                    ));
                }
                tracing::warn!("[{}] spotDL dir contents:\n{}", sub.id, listing);
            }
            let summary = if stdout.is_empty() {
                "spotDL exited OK but no output file found".into()
            } else {
                let truncated: String = stdout.lines().take(3).collect::<Vec<_>>().join(" | ");
                format!("spotDL OK but no file — output: {truncated}")
            };
            note(pool, sub.id, "spotDL", &summary).await;
        } else {
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
    best_dir: &Path,
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

                match done(pool, dir, best_dir, id, &name, "yt-dlp").await {
                    Ok(()) => return Ok(()),
                    Err(e) => {
                        // Verification rejected — retry if attempts remain
                        let rsn = format!("{e:#}");
                        note(pool, id, "yt-dlp", &format!("attempt {a} rejected: {rsn}")).await;
                        if a < max_retries {
                            let delay = std::time::Duration::from_secs(2u64.pow(a - 1));
                            tracing::warn!("[{id}] yt-dlp {a} rejected ({delay:?}): {rsn}");
                            tokio::time::sleep(delay).await;
                            continue;
                        }
                        anyhow::bail!(
                            "yt-dlp verification rejected after {max_retries} attempts: {rsn}"
                        );
                    }
                }
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
    best_dir: &Path,
    id: i64,
    name: &str,
    stage: &str,
) -> anyhow::Result<()> {
    let full = dir.join(name);
    let sz = tokio::fs::metadata(&full)
        .await
        .ok()
        .map(|m| m.len() as i64);
    let container = name.split('.').last().map(|e| e.to_lowercase());
    let bitrate = extract_bitrate(&full).await.ok().flatten();

    // ── Verify file metadata against submission ──
    let sub_title: Option<String> =
        sqlx::query_scalar("SELECT track_title FROM submissions WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let sub_artist: Option<String> =
        sqlx::query_scalar("SELECT track_artist FROM submissions WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let sub_isrc: Option<String> = sqlx::query_scalar("SELECT isrc FROM submissions WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    let sub_duration_ms: Option<i64> =
        sqlx::query_scalar("SELECT duration_ms FROM submissions WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();

    match extract_metadata(&full).await {
        Ok(meta) => {
            // ISRC verification (primary — exact match)
            if let (Some(file_isrc), Some(exp_isrc)) = (&meta.isrc, &sub_isrc) {
                if !file_isrc.eq_ignore_ascii_case(exp_isrc) {
                    // Try cross-submission reassign
                    if let Ok(Some(correct_id)) = sqlx::query_scalar::<_, i64>(
                        "SELECT id FROM submissions WHERE isrc = ? AND id != ?",
                    )
                    .bind(file_isrc)
                    .bind(id)
                    .fetch_optional(pool)
                    .await
                    {
                        tracing::warn!(
                            "[{id}] ISRC mismatch: file={file_isrc} sub={exp_isrc} → reassigning to {correct_id}"
                        );
                        let c = name.split('.').last().map(|e| e.to_lowercase());
                        let corr_bitrate = extract_bitrate(&full).await.ok().flatten();
                        let corr_note = format!("downloaded via {stage} (ISRC-corrected)");
                        let _ = db::update_submission_status(
                            pool,
                            correct_id,
                            "ready",
                            Some(name),
                            sz,
                            Some(&corr_note),
                        )
                        .await;
                        let _ = sqlx::query("UPDATE submissions SET first_available_at = COALESCE(first_available_at, unixepoch()) WHERE id = ?").bind(correct_id).execute(pool).await;
                        let _ = db::append_attempt(
                            pool,
                            correct_id,
                            stage,
                            true,
                            Some(name),
                            corr_bitrate.as_deref(),
                            c.as_deref(),
                            None,
                        )
                        .await;
                        symlink_best(dir, best_dir, name, stage).await;
                        let _ = db::update_submission_status(
                            pool,
                            id,
                            "pending",
                            None,
                            None,
                            Some("ISRC mismatch — reassigned"),
                        )
                        .await;
                        anyhow::bail!(
                            "ISRC mismatch — file reassigned to submission {correct_id}, this submission falling through"
                        );
                    }
                    // No other submission has this ISRC → REJECT
                    return reject_file(
	                        &full,
	                        id,
	                        &format!(
	                            "ISRC mismatch: file has '{file_isrc}', expected '{exp_isrc}' (no matching submission)"
	                        ),
	                    )
	                    .await;
                }
            }

            // Title/artist verification (fallback when ISRC unavailable)
            if meta.isrc.is_none() {
                if let (Some(ft), Some(st)) = (&meta.title, &sub_title) {
                    if !titles_match(ft, st) {
                        return reject_file(
                            &full,
                            id,
                            &format!("title mismatch: file='{ft}', expected='{st}'"),
                        )
                        .await;
                    }
                }
            }

            // Artist verification — always run for non-yt-dlp sources.
            // ISRC can match but point to a different artist (e.g. Gippeul vs Fortuna
            // on Deezer for the same ISRC). Reject so spotDL can find the right one.
            if stage != "yt-dlp" {
                if let (Some(fa), Some(sa)) = (&meta.artist, &sub_artist) {
                    if !titles_match(fa, sa) {
                        return reject_file(
                            &full,
                            id,
                            &format!("artist mismatch: file='{fa}', expected='{sa}'"),
                        )
                        .await;
                    }
                }
            }

            // Duration verification — file duration must be within ±25% of expected
            if let Some(expected_ms) = sub_duration_ms {
                if let Ok(file_duration_s) = extract_duration(&full).await {
                    let file_ms = (file_duration_s * 1000.0) as i64;
                    let ratio = file_ms as f64 / expected_ms as f64;
                    if ratio < 0.75 || ratio > 1.25 {
                        return reject_file(
                            &full,
                            id,
                            &format!(
                                "duration mismatch: file={:.0}s, expected={:.0}s ({:.0}%)",
                                file_duration_s,
                                expected_ms as f64 / 1000.0,
                                ratio * 100.0
                            ),
                        )
                        .await;
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!("[{id}] metadata extraction failed: {e} — proceeding unverified");
        }
    }

    // ── Verification passed — mark as ready ──
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
        bitrate.as_deref(),
        container.as_deref(),
        None,
    )
    .await;
    tracing::info!("[{id}] ready [{stage}] {name}");

    symlink_best(dir, best_dir, name, stage).await;

    Ok(())
}

/// Strip the __w{id}__ prefix from spotDL filenames for clean symlink names.
fn clean_name<'a>(name: &'a str, stage: &str) -> &'a str {
    if stage == "deemix" {
        return name;
    }
    // spotDL files are named __w{id}__{title} - {artists}.mp3
    // yt-dlp files may also have the prefix if they came through the spotDL path
    if let Some(rest) = name.strip_prefix("__w") {
        if let Some(idx) = rest.find("__") {
            return &rest[idx + 2..];
        }
    }
    name
}

/// Create a symlink from best/{clean_name} → ../{source_dir}/{original_name}.
/// Deemix always wins over spotdl/yt-dlp for the same filename.
async fn symlink_best(source_dir: &Path, best_dir: &Path, name: &str, stage: &str) {
    let symlink_name = clean_name(name, stage);
    let link = best_dir.join(symlink_name);
    let source_component = source_dir
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_else(|| "unknown".into());
    let target = format!("../{source_component}/{name}");

    // Deemix always overwrites spotdl/yt-dlp symlinks for the same filename.
    // spotdl/yt-dlp never overwrite an existing deemix symlink.
    let is_deemix = stage == "deemix";
    if !is_deemix && link.exists() {
        // Check if the existing symlink points to deemix — if so, don't overwrite
        if let Ok(existing) = std::fs::read_link(&link) {
            if existing.to_string_lossy().contains("/deemix/") {
                tracing::info!(
                    "keeping deemix symlink for {symlink_name} (spotdl/yt-dlp version ignored)"
                );
                return;
            }
        }
    }

    // Remove existing symlink or file
    if link.exists() {
        let _ = std::fs::remove_file(&link);
    }

    match std::os::unix::fs::symlink(&target, &link) {
        Ok(()) => tracing::info!("symlink: best/{symlink_name} → {target}"),
        Err(e) => tracing::warn!("failed to symlink best/{symlink_name}: {e}"),
    }
}

async fn fail(pool: &SqlitePool, id: i64, msg: &str) {
    let _ = db::update_submission_status(pool, id, "failed", None, None, Some(msg)).await;
    let _ = db::append_attempt(pool, id, "fail", false, None, None, None, Some(msg)).await;
    tracing::error!("[{id}] FAILED: {msg}");
}

/// Find a file in dir whose name starts with the given prefix.
async fn find_by_prefix(dir: &Path, prefix: &str) -> Option<String> {
    const AUDIO_EXTS: &[&str] = &[".mp3", ".flac", ".m4a", ".opus", ".webm", ".ogg"];
    let mut entries = tokio::fs::read_dir(dir).await.ok()?;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(prefix) && AUDIO_EXTS.iter().any(|ext| name.ends_with(ext)) {
            return Some(name);
        }
    }
    None
}

/// Metadata extracted from an audio file via ffprobe.
/// Extract audio bitrate from a file via ffprobe.
/// Returns e.g. "320kbps" or None if ffprobe fails or returns unusable data.
async fn extract_bitrate(path: &Path) -> anyhow::Result<Option<String>> {
    let output = tokio::process::Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-show_entries",
            "format=bit_rate",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .await?;

    if output.status.success() {
        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if let Ok(bps) = raw.parse::<f64>() {
            if bps > 0.0 {
                let kbps = (bps / 1000.0).round() as u32;
                return Ok(Some(format!("{kbps}kbps")));
            }
        }
    }
    Ok(None)
}

/// Metadata extracted from an audio file via ffprobe.
struct FileMetadata {
    title: Option<String>,
    artist: Option<String>,
    isrc: Option<String>,
}

/// Extract title, artist, and ISRC from an audio file via ffprobe.
async fn extract_metadata(path: &Path) -> anyhow::Result<FileMetadata> {
    let output = tokio::process::Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-show_entries",
            "format_tags=title,artist,TSRC",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .await?;

    if !output.status.success() {
        anyhow::bail!("ffprobe exited non-zero");
    }

    let v: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("ffprobe JSON parse")?;
    let tags = v.get("format").and_then(|f| f.get("tags"));

    Ok(FileMetadata {
        title: tags
            .and_then(|t| t.get("title"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        artist: tags
            .and_then(|t| t.get("artist"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        isrc: tags
            .and_then(|t| t.get("TSRC"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    })
}

/// Check if file and expected titles/artists match. Uses normalized exact match
/// first, then substring containment (handles YouTube prefixes like
/// "mao zedong propaganda music Red Sun in the Sky").
fn titles_match(file_str: &str, expected: &str) -> bool {
    let f = normalize_for_match(file_str);
    let e = normalize_for_match(expected);
    if f == e {
        return true;
    }
    // Substring match — expected title must be at least 5 chars to avoid
    // false positives like "Red" matching "Red Sun In The Sky".
    e.len() >= 5 && f.contains(&e)
}

/// Extract audio duration in seconds from a file via ffprobe.
async fn extract_duration(path: &Path) -> anyhow::Result<f64> {
    let output = tokio::process::Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .await?;

    if output.status.success() {
        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return raw.parse::<f64>().context("parse duration");
    }
    anyhow::bail!("ffprobe duration failed");
}

/// Normalize a title/artist string for fuzzy comparison.
fn normalize_for_match(s: &str) -> String {
    let s = s.to_lowercase().trim().to_string();
    for suffix in &[
        "(remastered)",
        "[remastered]",
        "- remastered",
        "(live)",
        "[live]",
        "- live",
        "(remaster)",
        "- remaster",
        "(original mix)",
        "(radio edit)",
        "- radio edit",
        "(single version)",
        "[single version]",
    ] {
        if let Some(stripped) = s.strip_suffix(suffix) {
            return stripped.trim().to_string();
        }
    }
    s
}

/// Reject a wrongly-downloaded file: quarantine and return an error so the
/// pipeline falls through to the next download layer.
async fn reject_file(full: &Path, id: i64, reason: &str) -> anyhow::Result<()> {
    if let Some(parent) = full.parent() {
        let rejected_dir = parent.join("_rejected");
        let _ = tokio::fs::create_dir_all(&rejected_dir).await;
        if let Some(fname) = full.file_name() {
            let dest = rejected_dir.join(fname);
            if let Err(e) = tokio::fs::rename(full, &dest).await {
                tracing::warn!("[{id}] failed to quarantine rejected file: {e} — deleting instead");
                let _ = tokio::fs::remove_file(full).await;
            } else {
                tracing::info!("[{id}] quarantined: {}", dest.display());
            }
        }
    }

    tracing::warn!("[{id}] REJECTED: {reason}");
    anyhow::bail!("download rejected: {reason}");
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
