use anyhow::Result;
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};

/// Database backend type for the relay
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseBackend {
    /// LMDB backend (persistent, general purpose)
    #[default]
    Lmdb,
    /// NostrDB backend (persistent, optimized for Nostr)
    NostrDb,
    /// In-memory database (fastest, no persistence - uses temp directory for git data)
    Memory,
}

impl std::fmt::Display for DatabaseBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Memory => write!(f, "memory"),
            Self::NostrDb => write!(f, "nostrdb"),
            Self::Lmdb => write!(f, "lmdb"),
        }
    }
}

/// ngit-grasp - A GRASP (Git Relays Authorized via Signed-Nostr Proofs) implementation
///
/// Configuration is loaded with the following priority (highest to lowest):
/// 1. CLI flags (e.g., --domain example.com)
/// 2. Environment variables (e.g., NGIT_DOMAIN=example.com)
/// 3. .env file (loaded automatically if present)
/// 4. Built-in defaults
#[derive(Debug, Clone, Serialize, Deserialize, Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Config {
    /// Domain where this instance is hosted (required, used in GRASP validation)
    #[arg(long, env = "NGIT_DOMAIN")]
    pub domain: String,

    /// Owner's npub (optional, for relay info in NIP-11)
    #[arg(long, env = "NGIT_OWNER_NPUB")]
    pub owner_npub: Option<String>,

    /// Relay name for NIP-11 information document (defaults to "${domain} grasp relay")
    #[arg(long = "relay-name", env = "NGIT_RELAY_NAME")]
    pub relay_name_override: Option<String>,

    /// Relay description for NIP-11 information document
    #[arg(
        long,
        env = "NGIT_RELAY_DESCRIPTION",
        default_value = "Git Nostr Relay - a grasp implementation"
    )]
    pub relay_description: String,

    /// Path to store Git repositories
    #[arg(long, env = "NGIT_GIT_DATA_PATH", default_value = "./data/git")]
    pub git_data_path: String,

    /// Path to store Nostr relay data
    #[arg(long, env = "NGIT_RELAY_DATA_PATH", default_value = "./data/relay")]
    pub relay_data_path: String,

    /// Server bind address (IP:PORT)
    #[arg(long, env = "NGIT_BIND_ADDRESS", default_value = "127.0.0.1:8080")]
    pub bind_address: String,

    /// Database backend type
    #[arg(long, env = "NGIT_DATABASE_BACKEND", value_enum, default_value_t = DatabaseBackend::Lmdb)]
    pub database_backend: DatabaseBackend,

    /// Enable Prometheus metrics endpoint
    #[arg(long, env = "NGIT_METRICS_ENABLED", default_value_t = true)]
    pub metrics_enabled: bool,

    /// Connections per IP before flagging as potential abuse in metrics (display only, no rate limiting)
    #[arg(long = "metrics-connection-per-ip-abuse-threshold", env = "NGIT_METRICS_CONNECTION_PER_IP_ABUSE_THRESHOLD", default_value_t = 10)]
    pub metrics_connection_per_ip_abuse_threshold: u32,

    /// Number of top bandwidth repos to track in metrics
    #[arg(long = "metrics-top-n-repos", env = "NGIT_METRICS_TOP_N_REPOS", default_value_t = 10)]
    pub metrics_top_n_repos: usize,

    /// URL of bootstrap relay to sync from on startup (optional)
    /// Sync discovers additional relays from repository announcements that list our service
    #[arg(long, env = "NGIT_SYNC_BOOTSTRAP_RELAY_URL")]
    pub sync_bootstrap_relay_url: Option<String>,

    /// Maximum backoff time in seconds for sync relay reconnection (default: 3600 = 1 hour)
    #[arg(long, env = "NGIT_SYNC_MAX_BACKOFF_SECS", default_value_t = 3600)]
    pub sync_max_backoff_secs: u64,

    /// Delay in seconds before running startup catchup (default: 30)
    #[arg(long, env = "NGIT_SYNC_STARTUP_DELAY_SECS", default_value_t = 30)]
    pub sync_startup_delay_secs: u64,

    /// Delay in seconds before running reconnect catchup (default: 10)
    #[arg(long, env = "NGIT_SYNC_RECONNECT_DELAY_SECS", default_value_t = 10)]
    pub sync_reconnect_delay_secs: u64,

    /// Number of days to look back for reconnect catchup (default: 3)
    #[arg(long, env = "NGIT_SYNC_RECONNECT_LOOKBACK_DAYS", default_value_t = 3)]
    pub sync_reconnect_lookback_days: u64,

    /// Maximum startup jitter in milliseconds for sync connections (default: 10000 = 10 seconds)
    /// Set to 0 to disable jitter (useful for testing)
    #[arg(long, env = "NGIT_SYNC_STARTUP_JITTER_MS", default_value_t = 10_000)]
    pub sync_startup_jitter_ms: u64,

    /// Interval in seconds for checking disconnected relays and attempting reconnection (default: 60)
    /// Set to lower value for faster reconnection testing
    #[arg(long, env = "NGIT_SYNC_DISCONNECT_CHECK_INTERVAL_SECS", default_value_t = 60)]
    pub sync_disconnect_check_interval_secs: u64,

    /// Base backoff time in seconds for relay reconnection (default: 5)
    /// Used for exponential backoff: base * 2^(failures-1)
    /// Set to 1 for faster test cycles
    /// Note: The connection timeout is capped at this value
    #[arg(long, env = "NGIT_SYNC_BASE_BACKOFF_SECS", default_value_t = 5)]
    pub sync_base_backoff_secs: u64,
}

impl Config {
    /// Load configuration from CLI args, environment variables, and defaults.
    ///
    /// Priority (highest to lowest):
    /// 1. CLI flags
    /// 2. Environment variables
    /// 3. .env file
    /// 4. Built-in defaults
    pub fn load() -> Result<Self> {
        // Load .env file if present (before clap parses, so env vars are available)
        dotenvy::dotenv().ok();

        // Parse CLI args (clap automatically handles env var fallback)
        let config = Self::parse();

        Ok(config)
    }

    /// Get relay name (defaults to "${domain} grasp relay" if not set)
    pub fn relay_name(&self) -> String {
        self.relay_name_override
            .clone()
            .unwrap_or_else(|| format!("{} grasp relay", self.domain))
    }

    /// Get effective git data path
    /// Returns a temp directory when using memory backend, otherwise the configured path
    pub fn effective_git_data_path(&self) -> String {
        if self.database_backend == DatabaseBackend::Memory {
            std::env::temp_dir()
                .join("ngit-grasp-git")
                .to_string_lossy()
                .into_owned()
        } else {
            self.git_data_path.clone()
        }
    }

    /// Create config for testing
    #[cfg(test)]
    pub fn for_testing() -> Self {
        Self {
            domain: "localhost:8080".to_string(),
            owner_npub: Some("npub1test".to_string()),
            relay_name_override: Some("test relay".to_string()),
            relay_description: "test description".to_string(),
            git_data_path: "./test_data/git".to_string(),
            relay_data_path: "./test_data/relay".to_string(),
            bind_address: "127.0.0.1:8080".to_string(),
            database_backend: DatabaseBackend::Memory,
            metrics_enabled: true,
            metrics_connection_per_ip_abuse_threshold: 10,
            metrics_top_n_repos: 10,
            sync_bootstrap_relay_url: None,
            sync_max_backoff_secs: 3600,
            sync_startup_delay_secs: 30,
            sync_reconnect_delay_secs: 10,
            sync_reconnect_lookback_days: 3,
            sync_startup_jitter_ms: 10_000,
            sync_disconnect_check_interval_secs: 60,
            sync_base_backoff_secs: 5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_values() {
        let config = Config::for_testing();
        assert_eq!(config.domain, "localhost:8080");
        assert_eq!(config.bind_address, "127.0.0.1:8080");
        // for_testing() uses Memory, but the actual default is Lmdb
        assert_eq!(config.database_backend, DatabaseBackend::Memory);
    }

    #[test]
    fn test_lmdb_is_default() {
        // Verify the actual default via the enum's Default trait
        assert_eq!(DatabaseBackend::default(), DatabaseBackend::Lmdb);
    }

    #[test]
    fn test_memory_backend_uses_temp_dir() {
        let config = Config {
            database_backend: DatabaseBackend::Memory,
            ..Config::for_testing()
        };
        let git_path = config.effective_git_data_path();
        assert!(git_path.contains("ngit-grasp-git"));
    }

    #[test]
    fn test_lmdb_backend_uses_configured_path() {
        let config = Config {
            database_backend: DatabaseBackend::Lmdb,
            git_data_path: "./my/git/path".to_string(),
            relay_data_path: "./my/relay/path".to_string(),
            ..Config::for_testing()
        };
        assert_eq!(config.effective_git_data_path(), "./my/git/path");
    }

    #[test]
    fn test_database_backend_display() {
        assert_eq!(DatabaseBackend::Memory.to_string(), "memory");
        assert_eq!(DatabaseBackend::NostrDb.to_string(), "nostrdb");
        assert_eq!(DatabaseBackend::Lmdb.to_string(), "lmdb");
    }

    #[test]
    fn test_relay_name_default() {
        let config = Config {
            domain: "example.com".to_string(),
            relay_name_override: None,
            ..Config::for_testing()
        };
        assert_eq!(config.relay_name(), "example.com grasp relay");
    }

    #[test]
    fn test_relay_name_override() {
        let config = Config {
            domain: "example.com".to_string(),
            relay_name_override: Some("My Custom Relay".to_string()),
            ..Config::for_testing()
        };
        assert_eq!(config.relay_name(), "My Custom Relay");
    }

    #[test]
    fn test_owner_npub_optional() {
        let config = Config {
            owner_npub: None,
            ..Config::for_testing()
        };
        assert!(config.owner_npub.is_none());
    }

    #[test]
    fn test_metrics_config_defaults() {
        let config = Config::for_testing();
        assert!(config.metrics_enabled);
        assert_eq!(config.metrics_connection_per_ip_abuse_threshold, 10);
        assert_eq!(config.metrics_top_n_repos, 10);
    }

    #[test]
    fn test_metrics_config_custom_values() {
        let config = Config {
            metrics_enabled: false,
            metrics_connection_per_ip_abuse_threshold: 50,
            metrics_top_n_repos: 25,
            ..Config::for_testing()
        };
        assert!(!config.metrics_enabled);
        assert_eq!(config.metrics_connection_per_ip_abuse_threshold, 50);
        assert_eq!(config.metrics_top_n_repos, 25);
    }
}
