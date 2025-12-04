use std::sync::Arc;

use anyhow::Result;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

use ngit_grasp::{
    config::{Config, DatabaseBackend},
    http,
    metrics::Metrics,
    nostr,
    sync::SyncManager,
};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("Starting ngit-grasp with nostr-relay-builder...");

    // Load configuration (priority: CLI flags > env vars > .env file > defaults)
    let config = Config::load()?;

    info!("Configuration loaded: {}", config.bind_address);
    info!("Domain: {}", config.domain);
    info!("Relay name: {}", config.relay_name());
    info!("Git data directory: {}", config.effective_git_data_path());
    if config.database_backend != DatabaseBackend::Memory {
        info!("Relay data directory: {}", config.relay_data_path);
    }
    info!("Database backend: {}", config.database_backend);

    // Initialize metrics if enabled
    let metrics = if config.metrics_enabled {
        info!("Metrics enabled on /metrics endpoint");
        Some(Arc::new(Metrics::new(config.metrics_connection_per_ip_abuse_threshold)))
    } else {
        info!("Metrics disabled");
        None
    };

    // Create Nostr relay with NIP-34 validation
    // Returns both the relay and database for direct queries in handlers
    if let Ok(relay_with_db) = nostr::builder::create_relay(&config) {
        info!(
            "Relay created with NIP-34 validation for domain: {}",
            config.domain
        );

        // Start SyncManager for proactive sync (Phase 2: multi-relay support)
        // Even without initial sync_relay_url, SyncManager can discover relays from stored announcements
        let sync_manager = SyncManager::new(
            config.sync_relay_url.clone(),
            config.domain.clone(),
            relay_with_db.database.clone(),
            relay_with_db.write_policy.clone(),
        );

        if config.sync_relay_url.is_some() {
            info!("Starting proactive sync from: {:?}", config.sync_relay_url);
        } else {
            info!("Proactive sync enabled (will discover relays from stored announcements)");
        }

        tokio::spawn(async move {
            sync_manager.run().await;
        });

        // Start HTTP server with integrated relay and database
        info!("Starting HTTP server on {}", config.bind_address);
        http::run_server(config, relay_with_db.relay, relay_with_db.database, metrics).await?;
    }

    Ok(())
}
