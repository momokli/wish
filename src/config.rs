use anyhow::Context;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SpotifyConfig {
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DeemixConfig {
    #[serde(default = "default_deemix_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub arl: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DownloadConfig {
    #[serde(default = "default_output_dir")]
    pub output_dir: PathBuf,
    #[serde(default = "default_max_per_user")]
    pub max_per_user: u32,
    #[serde(default)]
    pub ytdlp_cookies: Option<PathBuf>,
    #[serde(default)]
    pub ytdlp_proxy: Option<String>,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u32,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_download_timeout_secs")]
    pub download_timeout_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub spotify: SpotifyConfig,
    #[serde(default)]
    pub deemix: DeemixConfig,
    #[serde(default)]
    pub download: DownloadConfig,
}

fn default_deemix_base_url() -> String {
    "http://localhost:6595".to_string()
}

fn default_output_dir() -> PathBuf {
    PathBuf::from("./downloads")
}

fn default_max_per_user() -> u32 {
    5
}

fn default_max_concurrent() -> u32 {
    3
}

fn default_max_retries() -> u32 {
    3
}

fn default_download_timeout_secs() -> u64 {
    300
}

impl Config {
    /// Load config with priority: env vars > `~/.config/wish/config.toml` > defaults.
    pub fn load() -> anyhow::Result<Self> {
        // Start with defaults
        let mut config = Config::default();

        // Layer 1: config.toml from ~/.config/wish/
        if let Some(config_dir) = dirs::config_dir() {
            let config_path = config_dir.join("wish").join("config.toml");
            if config_path.exists() {
                let contents = std::fs::read_to_string(&config_path)
                    .with_context(|| format!("Failed to read config: {}", config_path.display()))?;
                let file_config: Config = toml::from_str(&contents).with_context(|| {
                    format!("Failed to parse config: {}", config_path.display())
                })?;
                config = file_config;
            }
        }

        // Layer 2: .env file (lowest env priority)
        let _ = dotenvy::dotenv();

        // Layer 3: environment variables (highest priority)
        if let Ok(val) = std::env::var("WISH_SPOTIFY_CLIENT_ID") {
            config.spotify.client_id = val;
        }
        if let Ok(val) = std::env::var("WISH_SPOTIFY_CLIENT_SECRET") {
            config.spotify.client_secret = val;
        }
        if let Ok(val) = std::env::var("WISH_DEEMIX_BASE_URL") {
            config.deemix.base_url = val;
        }
        if let Ok(val) = std::env::var("WISH_DEEMIX_ARL") {
            config.deemix.arl = val;
        }
        if let Ok(val) = std::env::var("WISH_DOWNLOAD_OUTPUT_DIR") {
            config.download.output_dir = PathBuf::from(val);
        }
        if let Ok(val) = std::env::var("WISH_DOWNLOAD_MAX_PER_USER") {
            config.download.max_per_user = val.parse().unwrap_or(default_max_per_user());
        }
        if let Ok(val) = std::env::var("WISH_DOWNLOAD_YTDLP_COOKIES") {
            config.download.ytdlp_cookies = Some(PathBuf::from(val));
        }
        if let Ok(val) = std::env::var("WISH_DOWNLOAD_YTDLP_PROXY") {
            config.download.ytdlp_proxy = Some(val);
        }
        if let Ok(val) = std::env::var("WISH_DOWNLOAD_MAX_CONCURRENT") {
            config.download.max_concurrent = val.parse().unwrap_or(3);
        }
        if let Ok(val) = std::env::var("WISH_DOWNLOAD_MAX_RETRIES") {
            config.download.max_retries = val.parse().unwrap_or(3);
        }
        if let Ok(val) = std::env::var("WISH_DOWNLOAD_TIMEOUT_SECS") {
            config.download.download_timeout_secs = val.parse().unwrap_or(300);
        }

        // Ensure output directory exists
        if !config.download.output_dir.exists() {
            std::fs::create_dir_all(&config.download.output_dir).with_context(|| {
                format!(
                    "Failed to create output directory: {}",
                    config.download.output_dir.display()
                )
            })?;
        }

        tracing::info!(
            "Config loaded: spotify={}, deemix={}, output={}",
            if config.spotify.client_id.is_empty() {
                "unset"
            } else {
                "set"
            },
            config.deemix.base_url,
            config.download.output_dir.display()
        );

        Ok(config)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            spotify: SpotifyConfig {
                client_id: String::new(),
                client_secret: String::new(),
            },
            deemix: DeemixConfig {
                base_url: default_deemix_base_url(),
                arl: String::new(),
            },
            download: DownloadConfig {
                output_dir: default_output_dir(),
                max_per_user: default_max_per_user(),
                ytdlp_cookies: None,
                ytdlp_proxy: None,
                max_concurrent: default_max_concurrent(),
                max_retries: default_max_retries(),
                download_timeout_secs: default_download_timeout_secs(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(config.spotify.client_id.is_empty());
        assert_eq!(config.deemix.base_url, "http://localhost:6595");
        assert_eq!(config.download.max_per_user, 5);
        assert_eq!(config.download.output_dir, PathBuf::from("./downloads"));
    }
}
