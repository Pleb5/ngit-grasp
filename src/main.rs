use anyhow::Result;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

mod config;
mod nostr;
mod storage;

use config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("Starting ngit-grasp...");

    // Load configuration
    let config = Config::from_env()?;
    info!("Configuration loaded: {}", config.bind_address);

    // Initialize storage
    let storage = storage::Storage::new(&config)?;
    info!("Storage initialized at: {}", config.relay_data_path);

    // Start Nostr relay
    let relay = nostr::relay::RelayServer::new(config.clone(), storage)?;
    
    info!("Starting Nostr relay on {}", config.bind_address);
    relay.run().await?;

    Ok(())
}
