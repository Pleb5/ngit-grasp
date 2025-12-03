use crate::config::Config;
/// NIP-11 Relay Information Document
///
/// Implements NIP-11 relay information endpoint with GRASP-01 extensions.
/// See: https://github.com/nostr-protocol/nips/blob/master/11.md
use serde::{Deserialize, Serialize};

/// NIP-11 Relay Information Document
///
/// This structure represents the relay metadata served at the HTTP(S) endpoint
/// when the client sends `Accept: application/nostr+json` header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayInformationDocument {
    /// Relay name
    pub name: String,

    /// Relay description
    pub description: String,

    /// Relay owner's public key (hex format)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pubkey: Option<String>,

    /// Contact information for relay admin
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<String>,

    /// List of NIPs supported by this relay
    pub supported_nips: Vec<u16>,

    /// Relay software identifier
    pub software: String,

    /// Software version
    pub version: String,

    /// Relay icon URL (NIP-11 optional field)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    // GRASP-01 Extensions (lines 11-14 of GRASP-01 spec)
    /// List of supported GRASPs (e.g., ["GRASP-01"])
    /// Required by GRASP-01 specification line 12
    pub supported_grasps: Vec<String>,

    /// Repository acceptance criteria description
    /// Required by GRASP-01 specification line 13
    pub repo_acceptance_criteria: String,

    /// Curation policy (present if curated, absent otherwise)
    /// Optional per GRASP-01 specification line 14
    #[serde(skip_serializing_if = "Option::is_none")]
    pub curation: Option<String>,
}

impl RelayInformationDocument {
    /// Create NIP-11 relay information document from configuration
    pub fn from_config(config: &Config) -> Self {
        Self {
            name: config.relay_name(),
            description: config.relay_description.clone(),
            pubkey: config.owner_npub.clone(),
            contact: None, // Could be added to config if needed
            supported_nips: vec![
                1,  // NIP-01: Basic protocol flow
                11, // NIP-11: Relay information document (this!)
                34, // NIP-34: Git repository announcements
                77, // NIP-77: Negentropy sync (reconciliation protocol)
            ],
            software: "https://gitworkshop.dev/danconwaydev.com/ngit-grasp".to_string(),
            version: match option_env!("GIT_COMMIT_SHORT") {
                Some(commit) => format!("{}-{}", env!("CARGO_PKG_VERSION"), commit),
                None => env!("CARGO_PKG_VERSION").to_string(),
            },
            icon: Some(format!("https://{}/icon.png", config.domain)),

            // GRASP-01 Extensions
            supported_grasps: vec!["GRASP-01".to_string()],
            repo_acceptance_criteria: "None".to_string(),
            curation: None, // Not a curated relay - only SPAM prevention via GRASP-01 policy
        }
    }

    /// Serialize to JSON string
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relay_information_document_structure() {
        let config = Config {
            domain: "relay.example.com".to_string(),
            owner_npub: Some("npub1test".to_string()),
            relay_name_override: Some("Test Relay".to_string()),
            relay_description: "A test relay".to_string(),
            git_data_path: "./data/git".to_string(),
            relay_data_path: "./data/relay".to_string(),
            bind_address: "127.0.0.1:8080".to_string(),
            database_backend: crate::config::DatabaseBackend::Memory,
        };

        let doc = RelayInformationDocument::from_config(&config);

        assert_eq!(doc.name, "Test Relay");
        assert_eq!(doc.description, "A test relay");
        assert_eq!(doc.pubkey, Some("npub1test".to_string()));
        assert!(doc.supported_nips.contains(&1));
        assert!(doc.supported_nips.contains(&11));
        assert!(doc.supported_nips.contains(&34));
        assert!(doc.supported_nips.contains(&77));
        assert_eq!(doc.supported_grasps, vec!["GRASP-01"]);
        assert!(doc.repo_acceptance_criteria.contains("None"));
        assert!(doc.curation.is_none());
        assert_eq!(
            doc.icon,
            Some("https://relay.example.com/icon.png".to_string())
        );
    }

    #[test]
    fn test_relay_information_document_json() {
        let config = Config {
            domain: "relay.example.com".to_string(),
            owner_npub: Some("npub1test".to_string()),
            relay_name_override: Some("Test Relay".to_string()),
            relay_description: "A test relay".to_string(),
            git_data_path: "./data/git".to_string(),
            relay_data_path: "./data/relay".to_string(),
            bind_address: "127.0.0.1:8080".to_string(),
            database_backend: crate::config::DatabaseBackend::Memory,
        };

        let doc = RelayInformationDocument::from_config(&config);
        let json = doc.to_json().expect("Failed to serialize to JSON");

        // Verify JSON contains expected fields
        assert!(json.contains("\"name\""));
        assert!(json.contains("\"description\""));
        assert!(json.contains("\"supported_nips\""));
        assert!(json.contains("\"supported_grasps\""));
        assert!(json.contains("\"repo_acceptance_criteria\""));
        assert!(json.contains("GRASP-01"));

        // Verify it's valid JSON by parsing
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("Invalid JSON");
        assert_eq!(parsed["name"], "Test Relay");
        assert_eq!(parsed["supported_grasps"][0], "GRASP-01");
        assert_eq!(parsed["icon"], "https://relay.example.com/icon.png");
    }
}
