use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::env;

/// Database backend type for the relay
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseBackend {
    /// In-memory database (default, fastest, no persistence)
    Memory,
    /// NostrDB backend (persistent, optimized for Nostr)
    NostrDb,
    /// LMDB backend (persistent, general purpose)
    Lmdb,
}

impl Default for DatabaseBackend {
    fn default() -> Self {
        Self::Memory
    }
}

impl std::str::FromStr for DatabaseBackend {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "memory" => Ok(Self::Memory),
            "nostrdb" => Ok(Self::NostrDb),
            "lmdb" => Ok(Self::Lmdb),
            _ => Err(anyhow::anyhow!(
                "Invalid database backend: {}. Valid options: memory, nostrdb, lmdb",
                s
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub domain: String,
    pub owner_npub: String,
    pub relay_name: String,
    pub relay_description: String,
    pub git_data_path: String,
    pub relay_data_path: String,
    pub bind_address: String,
    pub database_backend: DatabaseBackend,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        // Load .env file if present
        dotenvy::dotenv().ok();

        // Parse database backend from environment
        let database_backend = env::var("NGIT_DATABASE_BACKEND")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default();

        Ok(Config {
            domain: env::var("NGIT_DOMAIN").unwrap_or_else(|_| "localhost:8080".to_string()),
            owner_npub: env::var("NGIT_OWNER_NPUB").context("NGIT_OWNER_NPUB must be set")?,
            relay_name: env::var("NGIT_RELAY_NAME")
                .unwrap_or_else(|_| "ngit-grasp relay".to_string()),
            relay_description: env::var("NGIT_RELAY_DESCRIPTION")
                .unwrap_or_else(|_| "A GRASP-compliant Nostr relay for Git".to_string()),
            git_data_path: env::var("NGIT_GIT_DATA_PATH")
                .unwrap_or_else(|_| "./data/git".to_string()),
            relay_data_path: env::var("NGIT_RELAY_DATA_PATH")
                .unwrap_or_else(|_| "./data/relay".to_string()),
            bind_address: env::var("NGIT_BIND_ADDRESS")
                .unwrap_or_else(|_| "127.0.0.1:8080".to_string()),
            database_backend,
        })
    }
}
