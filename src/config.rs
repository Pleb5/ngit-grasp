use anyhow::{anyhow, Context, Result};
use clap::{Parser, ValueEnum};
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// GRASP-05 Archive whitelist entry
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ArchiveWhitelistEntry {
    /// Archive all repos from this pubkey: "npub1..."
    Pubkey(String),

    /// Archive specific repo: "npub1.../identifier"
    Repository { npub: String, identifier: String },

    /// Archive any repo with this identifier: "identifier"
    Identifier(String),
}

impl ArchiveWhitelistEntry {
    /// Parse a whitelist entry from string
    ///
    /// Formats:
    /// - "npub1..." -> Pubkey
    /// - "npub1.../identifier" -> Repository
    /// - "identifier" -> Identifier
    ///
    /// Validates npub format at parse time (fail fast)
    pub fn parse(s: &str) -> Result<Self> {
        let trimmed = s.trim();

        if trimmed.contains('/') {
            // Format: npub1.../identifier
            let parts: Vec<&str> = trimmed.split('/').collect();
            if parts.len() != 2 {
                return Err(anyhow!(
                    "Invalid whitelist entry format '{}'. Expected 'npub/identifier'",
                    s
                ));
            }

            let npub = parts[0];
            let identifier = parts[1];

            // Validate npub format (fail fast)
            if !npub.starts_with("npub1") {
                return Err(anyhow!(
                    "Invalid whitelist entry '{}'. First part must be npub",
                    s
                ));
            }

            PublicKey::from_bech32(npub)
                .context(format!("Invalid npub in whitelist entry '{}'", s))?;

            Ok(Self::Repository {
                npub: npub.to_string(),
                identifier: identifier.to_string(),
            })
        } else if trimmed.starts_with("npub1") {
            // Format: npub1...
            // Validate npub format (fail fast)
            PublicKey::from_bech32(trimmed)
                .context(format!("Invalid npub in whitelist entry '{}'", s))?;

            Ok(Self::Pubkey(trimmed.to_string()))
        } else {
            // Format: identifier
            Ok(Self::Identifier(trimmed.to_string()))
        }
    }

    /// Check if this entry matches the given npub and identifier
    pub fn matches(&self, npub: &str, identifier: &str) -> bool {
        match self {
            Self::Pubkey(p) => npub == p,
            Self::Repository {
                npub: p,
                identifier: i,
            } => npub == p && identifier == i,
            Self::Identifier(i) => identifier == i,
        }
    }
}

/// GRASP-05 Archive mode configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveConfig {
    /// Accept all repository announcements (no filtering)
    ///
    /// WARNING: Setting this to true allows anyone to mirror any repository
    /// to this relay, potentially causing storage/bandwidth exhaustion.
    pub archive_all: bool,

    /// Whitelist entries for selective archiving
    ///
    /// If empty and archive_all is false, GRASP-05 is disabled (GRASP-01 strict mode).
    pub whitelist: Vec<ArchiveWhitelistEntry>,

    /// Read-only archive mode: relay is a read-only sync of archived repositories
    ///
    /// When true, the relay ONLY accepts announcements matching the archive whitelist/all.
    /// Announcements listing the relay but not in the whitelist are rejected.
    /// When false, the relay operates in GRASP-01 mode for unwhitelisted repos.
    pub read_only: bool,
}

impl ArchiveConfig {
    /// Check if GRASP-05 is enabled (either archive_all or non-empty whitelist)
    pub fn enabled(&self) -> bool {
        self.archive_all || !self.whitelist.is_empty()
    }

    /// Check if an announcement matches the archive configuration
    ///
    /// Returns true if:
    /// - archive_all is true, OR
    /// - announcement matches any whitelist entry
    pub fn matches(&self, npub: &str, identifier: &str) -> bool {
        if self.archive_all {
            return true;
        }

        self.whitelist
            .iter()
            .any(|entry| entry.matches(npub, identifier))
    }

    /// Parse archive whitelist from comma-separated string
    pub fn parse_whitelist(input: &str) -> Result<Vec<ArchiveWhitelistEntry>> {
        if input.trim().is_empty() {
            return Ok(Vec::new());
        }

        input
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(ArchiveWhitelistEntry::parse)
            .collect()
    }
}

impl Default for ArchiveConfig {
    fn default() -> Self {
        Self {
            archive_all: false,
            whitelist: Vec::new(),
            read_only: false,
        }
    }
}

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

    /// Relay operator's nsec (private key) for signing and authentication
    ///
    /// Used for:
    /// - NIP-11 relay information document (pubkey field derived from this nsec)
    /// - NIP-42 authentication when syncing from other relays
    /// - Future: signing events, WoT-based rate limiting of syncing relays
    ///
    /// If not provided via CLI/env, will be loaded from/saved to `.relay-owner.nsec` file
    /// in the current directory. If the file doesn't exist, a new key will be generated
    /// and saved automatically.
    #[arg(long, env = "NGIT_RELAY_OWNER_NSEC")]
    pub relay_owner_nsec: Option<String>,

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
    #[arg(
        long = "metrics-connection-per-ip-abuse-threshold",
        env = "NGIT_METRICS_CONNECTION_PER_IP_ABUSE_THRESHOLD",
        default_value_t = 10
    )]
    pub metrics_connection_per_ip_abuse_threshold: u32,

    /// Number of top bandwidth repos to track in metrics
    #[arg(
        long = "metrics-top-n-repos",
        env = "NGIT_METRICS_TOP_N_REPOS",
        default_value_t = 10
    )]
    pub metrics_top_n_repos: usize,

    /// URL of bootstrap relay to sync from on startup (optional)
    /// Sync discovers additional relays from repository announcements that list our service
    /// If no scheme is provided (wss:// or ws://), wss:// is assumed
    /// Examples: "relay.example.com" -> "wss://relay.example.com", "wss://relay.example.com" -> unchanged
    #[arg(long, env = "NGIT_SYNC_BOOTSTRAP_RELAY_URL")]
    pub sync_bootstrap_relay_url: Option<String>,

    /// Maximum backoff time in seconds for sync relay reconnection (default: 3600 = 1 hour)
    #[arg(long, env = "NGIT_SYNC_MAX_BACKOFF_SECS", default_value_t = 3600)]
    pub sync_max_backoff_secs: u64,

    /// Interval in seconds for checking disconnected relays and attempting reconnection (default: 60)
    /// Set to lower value for faster reconnection testing
    #[arg(
        long,
        env = "NGIT_SYNC_DISCONNECT_CHECK_INTERVAL_SECS",
        default_value_t = 60
    )]
    pub sync_disconnect_check_interval_secs: u64,

    /// Base backoff time in seconds for relay reconnection (default: 5)
    /// Used for exponential backoff: base * 2^(failures-1)
    /// Set to 1 for faster test cycles
    /// Note: The connection timeout is capped at this value
    #[arg(long, env = "NGIT_SYNC_BASE_BACKOFF_SECS", default_value_t = 5)]
    pub sync_base_backoff_secs: u64,

    /// Disable NIP-77 negentropy sync (default: false)
    /// When enabled, sync will use REQ+EOSE instead of negentropy for history sync.
    /// Primarily useful for testing that sync works without negentropy support.
    #[arg(long, env = "NGIT_SYNC_DISABLE_NEGENTROPY", default_value_t = false)]
    pub sync_disable_negentropy: bool,

    /// Hot cache duration in seconds for rejected announcements (default: 120 = 2 minutes)
    /// Stores full event objects for immediate re-processing when dependencies resolve.
    /// Too short (<30s): Miss events from slow relays
    /// Too long (>5min): Waste memory
    #[arg(
        long,
        env = "NGIT_REJECTED_HOT_CACHE_DURATION_SECS",
        default_value_t = 120
    )]
    pub rejected_hot_cache_duration_secs: u64,

    /// Cold index expiry in seconds for rejected announcements (default: 604800 = 7 days)
    /// Stores metadata only to prevent repeated downloads of rejected events.
    #[arg(
        long,
        env = "NGIT_REJECTED_COLD_INDEX_EXPIRY_SECS",
        default_value_t = 604800
    )]
    pub rejected_cold_index_expiry_secs: u64,

    /// Hours before removing relay from naughty list (default: 12)
    /// Relays with persistent infrastructure issues (DNS, TLS, protocol errors) are
    /// tracked separately and retried after this expiration period.
    #[arg(long, env = "NGIT_NAUGHTY_LIST_EXPIRATION_HOURS", default_value_t = 12)]
    pub naughty_list_expiration_hours: u64,

    /// Enable GRASP-05 archive mode: accept all announcements regardless of listing (WARNING: storage risk)
    #[arg(long, env = "NGIT_ARCHIVE_ALL", default_value_t = false)]
    pub archive_all: bool,

    /// GRASP-05 archive whitelist: comma-separated list of npub/identifier/npub/identifier entries
    /// Formats: "npub1...", "npub1.../identifier", "identifier"
    #[arg(long, env = "NGIT_ARCHIVE_WHITELIST", default_value = "")]
    pub archive_whitelist: String,

    /// Archive read-only mode: relay is a read-only sync of archived repositories
    /// Defaults to true if archive_all or archive_whitelist is set, false otherwise
    /// Throws error if set to true without archive_all or archive_whitelist
    #[arg(long, env = "NGIT_ARCHIVE_READ_ONLY")]
    pub archive_read_only: Option<bool>,
}

impl Config {
    /// Path to the relay owner key file
    const RELAY_OWNER_KEY_FILE: &'static str = ".relay-owner.nsec";

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
        let mut config = Self::parse();

        // If relay_owner_nsec not provided, load from file or generate
        if config.relay_owner_nsec.is_none() {
            config.relay_owner_nsec = Some(Self::load_or_generate_relay_owner_key()?);
        } else {
            // If provided via CLI/env, trim any whitespace (newlines, spaces, etc.)
            // This handles cases where the value is read from a file with trailing newline
            config.relay_owner_nsec = config.relay_owner_nsec.map(|s| s.trim().to_string());
        }

        Ok(config)
    }

    /// Load relay owner key from file, or generate and save a new one
    fn load_or_generate_relay_owner_key() -> Result<String> {
        let key_path = PathBuf::from(Self::RELAY_OWNER_KEY_FILE);

        // Try to load existing key
        if key_path.exists() {
            let nsec = fs::read_to_string(&key_path)
                .context("Failed to read relay owner key file")?
                .trim()
                .to_string();

            // Validate it's a valid nsec
            Keys::parse(&nsec).context("Invalid nsec in relay owner key file")?;

            tracing::info!("Loaded relay owner key from {}", key_path.display());
            return Ok(nsec);
        }

        // Generate new key
        let keys = Keys::generate();
        let nsec = keys.secret_key().to_bech32()?;

        // Save to file
        fs::write(&key_path, &nsec).context("Failed to write relay owner key file")?;

        tracing::info!(
            "Generated new relay owner key and saved to {}",
            key_path.display()
        );

        Ok(nsec)
    }

    /// Get the relay owner's Keys object
    pub fn relay_owner_keys(&self) -> Result<Keys> {
        let nsec = self
            .relay_owner_nsec
            .as_ref()
            .context("relay_owner_nsec not set (should be set by Config::load())")?;
        Keys::parse(nsec).context("Invalid relay_owner_nsec")
    }

    /// Get the relay owner's public key (npub format) for NIP-11
    pub fn relay_owner_npub(&self) -> Result<String> {
        let keys = self.relay_owner_keys()?;
        Ok(keys.public_key().to_bech32()?)
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

    /// Get parsed archive configuration with computed read-only mode
    ///
    /// Read-only mode defaults to true if archive mode is enabled, false otherwise.
    /// Throws error if explicitly set to true without archive mode enabled.
    pub fn archive_config(&self) -> Result<ArchiveConfig> {
        let whitelist = ArchiveConfig::parse_whitelist(&self.archive_whitelist)?;
        let archive_enabled = self.archive_all || !whitelist.is_empty();

        let read_only = match self.archive_read_only {
            Some(true) => {
                if !archive_enabled {
                    return Err(anyhow!(
                        "NGIT_ARCHIVE_READ_ONLY=true requires either NGIT_ARCHIVE_ALL=true or NGIT_ARCHIVE_WHITELIST to be set"
                    ));
                }
                true
            }
            Some(false) => false,
            None => {
                // Default: true if archive mode enabled, false otherwise
                archive_enabled
            }
        };

        Ok(ArchiveConfig {
            archive_all: self.archive_all,
            whitelist,
            read_only,
        })
    }

    /// Create config for testing
    #[cfg(test)]
    pub fn for_testing() -> Self {
        // Generate a test key deterministically for consistent tests
        let keys = Keys::generate();
        let nsec = keys
            .secret_key()
            .to_bech32()
            .expect("Failed to generate test nsec");

        Self {
            domain: "localhost:8080".to_string(),
            relay_owner_nsec: Some(nsec),
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
            sync_disconnect_check_interval_secs: 60,
            sync_base_backoff_secs: 5,
            sync_disable_negentropy: false,
            rejected_hot_cache_duration_secs: 120,
            rejected_cold_index_expiry_secs: 604800,
            naughty_list_expiration_hours: 12,
            archive_all: false,
            archive_whitelist: String::new(),
            archive_read_only: None,
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
    fn test_relay_owner_keys() {
        let config = Config::for_testing();
        let keys = config.relay_owner_keys().expect("Should have valid keys");
        let npub = config.relay_owner_npub().expect("Should derive npub");

        // Verify the npub matches the keys
        assert_eq!(npub, keys.public_key().to_bech32().unwrap());
        assert!(npub.starts_with("npub1"));
    }

    #[test]
    fn test_relay_owner_nsec_trims_whitespace() {
        // Test that Config::load() trims whitespace from provided nsec
        // This simulates what happens when nsec is read from a file with trailing newline
        let nsec_clean = "nsec1rt5f3gfnktvd77fdarg00ff94l8j833y778ym3xlzkx89g9v5zvq4y2qee";
        let nsec_with_newline = format!("{}\n", nsec_clean);
        let nsec_with_spaces = format!("  {}  ", nsec_clean);

        // Test that trimming happens by directly creating config with whitespace
        let mut config1 = Config::for_testing();
        config1.relay_owner_nsec = Some(nsec_with_newline.clone());
        // Simulate what Config::load() does
        config1.relay_owner_nsec = config1.relay_owner_nsec.map(|s| s.trim().to_string());

        let mut config2 = Config::for_testing();
        config2.relay_owner_nsec = Some(nsec_with_spaces.clone());
        config2.relay_owner_nsec = config2.relay_owner_nsec.map(|s| s.trim().to_string());

        // Both should parse successfully after trimming
        assert!(config1.relay_owner_keys().is_ok());
        assert!(config2.relay_owner_keys().is_ok());

        // Both should produce the same public key
        let keys1 = config1.relay_owner_keys().unwrap();
        let keys2 = config2.relay_owner_keys().unwrap();
        assert_eq!(keys1.public_key(), keys2.public_key());

        // Verify trimmed nsec equals clean nsec
        assert_eq!(config1.relay_owner_nsec.unwrap(), nsec_clean);
        assert_eq!(config2.relay_owner_nsec.unwrap(), nsec_clean);
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

    #[test]
    fn test_parse_whitelist_entry_pubkey() {
        // Generate a valid test npub
        let keys = Keys::generate();
        let test_npub = keys.public_key().to_bech32().unwrap();
        let entry = ArchiveWhitelistEntry::parse(&test_npub).unwrap();
        assert!(matches!(entry, ArchiveWhitelistEntry::Pubkey(_)));
        if let ArchiveWhitelistEntry::Pubkey(npub) = entry {
            assert_eq!(npub, test_npub);
        }
    }

    #[test]
    fn test_parse_whitelist_entry_repository() {
        let keys = Keys::generate();
        let test_npub = keys.public_key().to_bech32().unwrap();
        let entry = ArchiveWhitelistEntry::parse(&format!("{}/linux", test_npub)).unwrap();
        assert!(matches!(entry, ArchiveWhitelistEntry::Repository { .. }));
        if let ArchiveWhitelistEntry::Repository { npub, identifier } = entry {
            assert_eq!(npub, test_npub);
            assert_eq!(identifier, "linux");
        }
    }

    #[test]
    fn test_parse_whitelist_entry_identifier() {
        let entry = ArchiveWhitelistEntry::parse("bitcoin-core").unwrap();
        assert!(matches!(entry, ArchiveWhitelistEntry::Identifier(_)));
        if let ArchiveWhitelistEntry::Identifier(id) = entry {
            assert_eq!(id, "bitcoin-core");
        }
    }

    #[test]
    fn test_parse_whitelist_entry_invalid_npub() {
        let result = ArchiveWhitelistEntry::parse("npub1invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_whitelist_entry_matches() {
        let keys = Keys::generate();
        let test_npub = keys.public_key().to_bech32().unwrap();
        let entry = ArchiveWhitelistEntry::Pubkey(test_npub.clone());
        assert!(entry.matches(&test_npub, "any-identifier"));
        assert!(!entry.matches("npub1different", "any-identifier"));
    }

    #[test]
    fn test_whitelist_entry_matches_repository() {
        let keys = Keys::generate();
        let test_npub = keys.public_key().to_bech32().unwrap();
        let entry = ArchiveWhitelistEntry::Repository {
            npub: test_npub.clone(),
            identifier: "linux".to_string(),
        };
        assert!(entry.matches(&test_npub, "linux"));
        assert!(!entry.matches(&test_npub, "bitcoin"));
        assert!(!entry.matches("npub1different", "linux"));
    }

    #[test]
    fn test_whitelist_entry_matches_identifier() {
        let entry = ArchiveWhitelistEntry::Identifier("bitcoin-core".to_string());
        assert!(entry.matches("npub1alice", "bitcoin-core"));
        assert!(entry.matches("npub1bob", "bitcoin-core"));
        assert!(!entry.matches("npub1alice", "other-repo"));
    }

    #[test]
    fn test_archive_config_enabled() {
        let config = ArchiveConfig::default();
        assert!(!config.enabled());

        let config = ArchiveConfig {
            archive_all: true,
            whitelist: Vec::new(),
            read_only: true,
        };
        assert!(config.enabled());

        let config = ArchiveConfig {
            archive_all: false,
            whitelist: vec![ArchiveWhitelistEntry::Identifier("test".into())],
            read_only: true,
        };
        assert!(config.enabled());
    }

    #[test]
    fn test_archive_config_matches() {
        let keys = Keys::generate();
        let test_npub = keys.public_key().to_bech32().unwrap();
        let config = ArchiveConfig {
            archive_all: false,
            whitelist: vec![
                ArchiveWhitelistEntry::Pubkey(test_npub.clone()),
                ArchiveWhitelistEntry::Identifier("bitcoin-core".into()),
            ],
            read_only: false,
        };

        assert!(config.matches(&test_npub, "any-repo"));
        assert!(config.matches("npub1bob", "bitcoin-core"));
        assert!(!config.matches("npub1bob", "other-repo"));
    }

    #[test]
    fn test_archive_config_matches_archive_all() {
        let config = ArchiveConfig {
            archive_all: true,
            whitelist: Vec::new(),
            read_only: true,
        };

        assert!(config.matches("npub1alice", "any-repo"));
        assert!(config.matches("npub1bob", "other-repo"));
    }

    #[test]
    fn test_parse_whitelist_empty() {
        let whitelist = ArchiveConfig::parse_whitelist("").unwrap();
        assert!(whitelist.is_empty());

        let whitelist = ArchiveConfig::parse_whitelist("   ").unwrap();
        assert!(whitelist.is_empty());
    }

    #[test]
    fn test_parse_whitelist_multiple() {
        let keys1 = Keys::generate();
        let keys2 = Keys::generate();
        let test_npub1 = keys1.public_key().to_bech32().unwrap();
        let test_npub2 = keys2.public_key().to_bech32().unwrap();
        let whitelist = ArchiveConfig::parse_whitelist(&format!(
            "{},bitcoin-core,{}/linux",
            test_npub1, test_npub2
        ))
        .unwrap();
        assert_eq!(whitelist.len(), 3);
    }

    #[test]
    fn test_archive_config_parsing() {
        let keys = Keys::generate();
        let test_npub = keys.public_key().to_bech32().unwrap();
        let config = Config {
            archive_whitelist: format!("{},bitcoin-core", test_npub),
            ..Config::for_testing()
        };
        let archive_config = config.archive_config().unwrap();
        assert_eq!(archive_config.whitelist.len(), 2);
    }

    #[test]
    fn test_archive_config_invalid_npub() {
        let config = Config {
            archive_whitelist: "npub1invalid".to_string(),
            ..Config::for_testing()
        };
        assert!(config.archive_config().is_err());
    }

    #[test]
    fn test_archive_read_only_defaults() {
        // Default: false when no archive mode
        let config = Config::for_testing();
        assert_eq!(config.archive_config().unwrap().read_only, false);

        // Default: true when archive_all is set
        let config = Config {
            archive_all: true,
            ..Config::for_testing()
        };
        assert_eq!(config.archive_config().unwrap().read_only, true);

        // Default: true when archive_whitelist is set
        let keys = Keys::generate();
        let test_npub = keys.public_key().to_bech32().unwrap();
        let config = Config {
            archive_whitelist: test_npub,
            ..Config::for_testing()
        };
        assert_eq!(config.archive_config().unwrap().read_only, true);
    }

    #[test]
    fn test_archive_read_only_explicit() {
        // Explicit true with archive_all
        let config = Config {
            archive_all: true,
            archive_read_only: Some(true),
            ..Config::for_testing()
        };
        assert_eq!(config.archive_config().unwrap().read_only, true);

        // Explicit false with archive_all (unusual but allowed)
        let config = Config {
            archive_all: true,
            archive_read_only: Some(false),
            ..Config::for_testing()
        };
        assert_eq!(config.archive_config().unwrap().read_only, false);

        // Explicit false without archive mode
        let config = Config {
            archive_read_only: Some(false),
            ..Config::for_testing()
        };
        assert_eq!(config.archive_config().unwrap().read_only, false);
    }

    #[test]
    fn test_archive_read_only_error() {
        // Error: true without archive mode
        let config = Config {
            archive_read_only: Some(true),
            ..Config::for_testing()
        };
        assert!(config.archive_config().is_err());
        assert!(config
            .archive_config()
            .unwrap_err()
            .to_string()
            .contains("requires either"));
    }
}
