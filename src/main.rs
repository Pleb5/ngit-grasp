use anyhow::Result;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

use ngit_grasp::{config::Config, http, nostr};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("Starting ngit-grasp with nostr-relay-builder...");

    // Load configuration
    let config = Config::from_env()?;
    info!("Configuration loaded: {}", config.bind_address);
    info!("Git data directory: {}", config.git_data_path);

    // Create Nostr relay with NIP-34 validation
    // Returns both the relay and database for direct queries in handlers
    if let Ok(relay_with_db) = nostr::builder::create_relay(&config) {
        info!(
            "Relay created with NIP-34 validation for domain: {}",
            config.domain
        );

        // Start HTTP server with integrated relay and database
        info!("Starting HTTP server on {}", config.bind_address);
        http::run_server(config, relay_with_db.relay, relay_with_db.database).await?;
    }

    Ok(())
}
