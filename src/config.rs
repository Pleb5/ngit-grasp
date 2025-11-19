use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub domain: String,
    pub owner_npub: String,
    pub relay_name: String,
    pub relay_description: String,
    pub git_data_path: String,
    pub relay_data_path: String,
    pub bind_address: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        // Load .env file if present
        dotenvy::dotenv().ok();

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
        })
    }
}
