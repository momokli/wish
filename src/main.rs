use clap::Parser;
use sqlx::sqlite::SqlitePoolOptions;
use std::sync::Arc;
use tokio::sync::Notify;
use wish::config::Config;

/// Wish — Song request server.
#[derive(Parser, Debug)]
#[command(name = "wish", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Parser, Debug)]
enum Command {
    /// Start the HTTP server.
    Serve {
        /// Port to listen on (default: 3000, overrides WISH_PORT env).
        #[arg(long, default_value_t = default_port())]
        port: u16,
    },
}

fn default_port() -> u16 {
    std::env::var("WISH_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "wish=info,tower_http=info".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Serve { port } => run_server(port).await,
    }
}

async fn run_server(port: u16) -> anyhow::Result<()> {
    // Load config
    let config = Config::load()?;

    // Connect to SQLite
    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:wish.db?mode=rwc".to_string());

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to database: {}", e))?;

    // Run migrations
    wish::db::run_migrations(&pool).await?;

    // Initialize Spotify client (optional — server still works without it)
    let spotify = if !config.spotify.client_id.is_empty() {
        match wish::spotify::SpotifyClient::new(
            &config.spotify.client_id,
            &config.spotify.client_secret,
        )
        .await
        {
            Ok(client) => {
                tracing::info!("Spotify client initialized");
                Some(client)
            }
            Err(e) => {
                tracing::warn!(
                    "Spotify client initialization failed: {}. Search will be unavailable.",
                    e
                );
                None
            }
        }
    } else {
        tracing::warn!("Spotify credentials not configured. Search will be unavailable.");
        None
    };

    // Check if yt-dlp is available on PATH
    let ytdlp_available = which::which("yt-dlp").is_ok();
    if ytdlp_available {
        tracing::info!("yt-dlp found on PATH — YouTube/SoundCloud search enabled");
    } else {
        tracing::warn!("yt-dlp not found on PATH — YouTube/SoundCloud search disabled");
    }

    // Set up download notification channel
    let download_notify = Arc::new(Notify::new());

    // Build app state
    let state = Arc::new(wish::api::AppState {
        pool: pool.clone(),
        config: config.clone(),
        spotify,
        download_notify: download_notify.clone(),
        ytdlp_available,
    });

    // Build router
    let app = wish::api::build_router(state);

    // Start the download worker in the background
    let deemix_client = wish::deemix::DeemixClient::new(config.deemix.base_url.clone());

    // Authenticate with deemix if ARL is configured
    if !config.deemix.arl.is_empty() {
        match deemix_client.login_arl(&config.deemix.arl).await {
            Ok(resp) => {
                let name = resp.user.name.as_deref().unwrap_or("unknown");
                let country = resp.user.country.as_deref().unwrap_or("?");
                let lossless = resp.user.can_stream_lossless.unwrap_or(false);
                tracing::info!(
                    "Deemix authenticated as {} ({} / lossless: {})",
                    name,
                    country,
                    lossless
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Deemix ARL login failed: {}. Downloads will skip deemix.",
                    e
                );
            }
        }
    } else {
        tracing::warn!("Deemix ARL not configured. Spotify downloads will skip deemix.");
    }

    let worker = wish::downloader::DownloadWorker::new(
        pool.clone(),
        deemix_client,
        config.download.output_dir.clone(),
        download_notify.clone(),
        ytdlp_available,
    );
    tokio::spawn(async move {
        worker.run().await;
    });

    // Start the server
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Wish server starting on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
