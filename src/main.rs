use std::time::Duration;
use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use tokio::signal;
use tracing::{error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;

use ngit_grasp::{
    config::{Config, DatabaseBackend},
    git, http,
    metrics::Metrics,
    nostr,
    purgatory::{sync::RealSyncContext, sync::ThrottleManager, Purgatory},
    sync::{naughty_list::NaughtyListTracker, SyncManager},
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

    // Validate configuration and fail fast on fatal errors
    // Recoverable issues (e.g., malformed whitelist entries) are logged as warnings
    config.validate()?;

    info!(
        "Configuration loaded and validated: {}",
        config.bind_address
    );
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
        let m = Arc::new(Metrics::new(
            config.metrics_connection_per_ip_abuse_threshold,
            Some(config.effective_git_data_path()),
        ));
        info!("Repository count will be updated on each metrics request");
        Some(m)
    } else {
        info!("Metrics disabled");
        None
    };

    // Create purgatory for event/git coordination
    let purgatory = Arc::new(Purgatory::new(PathBuf::from(
        config.effective_git_data_path(),
    )));
    info!("Purgatory initialized for event coordination");

    // Restore purgatory state from disk if available
    let purgatory_path =
        PathBuf::from(config.effective_git_data_path()).join("purgatory-state.json");

    if purgatory_path.exists() {
        match purgatory.restore_from_disk(&purgatory_path) {
            Ok(()) => {
                info!("Restored purgatory state from disk");
                // Re-queueing will happen later after sync system is created
            }
            Err(e) => {
                warn!("Failed to restore purgatory state: {}, starting empty", e);
            }
        }
    }

    // Create Nostr relay with NIP-34 validation
    // Returns both the relay and database for direct queries in handlers
    if let Ok(relay_with_db) = nostr::builder::create_relay(&config, purgatory.clone()).await {
        info!(
            "Relay created with NIP-34 validation for domain: {}",
            config.domain
        );

        // Set the local relay on the write policy for purgatory notifications
        // This must be done after relay creation since the relay depends on the policy
        relay_with_db
            .write_policy
            .set_local_relay(relay_with_db.relay.clone());

        // Start SyncManager for proactive sync (Phase 2: multi-relay support, Phase 3: health tracking)
        // Even without bootstrap relay, SyncManager discovers relays from stored announcements
        // Pass the already-registered sync metrics from Metrics to avoid duplicate registration
        let sync_manager = SyncManager::new(
            config.sync_bootstrap_relay_url.clone(),
            config.domain.clone(),
            relay_with_db.database.clone(),
            relay_with_db.write_policy.clone(),
            relay_with_db.relay.clone(),
            &config,
            PathBuf::from(config.effective_git_data_path()),
            metrics.as_ref().and_then(|m| m.sync_metrics().cloned()),
        );

        if config.sync_bootstrap_relay_url.is_some() {
            info!(
                "Starting proactive sync with bootstrap relay: {:?}",
                config.sync_bootstrap_relay_url
            );
        } else {
            info!("Proactive sync enabled (will discover relays from stored announcements)");
        }

        // Re-queue all restored purgatory repos for sync
        let restored_identifiers = purgatory.get_all_identifiers();
        if !restored_identifiers.is_empty() {
            info!(
                "Re-queueing {} restored repositories for sync",
                restored_identifiers.len()
            );
            for identifier in restored_identifiers {
                purgatory.enqueue_sync_immediate(&identifier);
            }
        }

        // Get a reference to the rejected events index for shutdown persistence
        let shutdown_rejected_index = sync_manager.rejected_events_index();

        tokio::spawn(async move {
            sync_manager.run().await;
        });

        // Spawn background cleanup task for purgatory entries (60s interval)
        let cleanup_purgatory = purgatory.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                let (state_removed, pr_removed) = cleanup_purgatory.cleanup();
                if state_removed > 0 || pr_removed > 0 {
                    info!(
                        "Purgatory cleanup: removed {} state events, {} PR events",
                        state_removed, pr_removed
                    );
                }
            }
        });
        info!("Purgatory cleanup task started (60s interval)");

        // Spawn daily cleanup task for old expired event records (prevent unbounded growth)
        let expired_cleanup_purgatory = purgatory.clone();
        tokio::spawn(async move {
            // Run immediately on startup, then every 24 hours
            let mut interval = tokio::time::interval(Duration::from_secs(24 * 3600));
            loop {
                interval.tick().await;
                // Remove expired event records older than 7 days
                let removed = expired_cleanup_purgatory
                    .cleanup_expired_events(Duration::from_secs(7 * 24 * 3600));
                if removed > 0 {
                    info!(
                        "Expired event cleanup: removed {} old expired event records (>7 days)",
                        removed
                    );
                }
            }
        });
        info!("Expired event cleanup task started (24h interval, keeps 7 days)");

        // Start purgatory sync loop for background git data fetching
        // Create naughty list tracker for git remote domains with persistent errors (12h expiration)
        let git_naughty_list = Arc::new(NaughtyListTracker::with_defaults());

        let sync_ctx = Arc::new(RealSyncContext::new(
            purgatory.clone(),
            relay_with_db.database.clone(),
            PathBuf::from(config.effective_git_data_path()),
            Some(config.domain.clone()),
            Some(relay_with_db.relay.clone()),
            git_naughty_list.clone(),
        ));

        // Create throttle manager for rate limiting remote git servers
        // Default: 5 concurrent requests per domain, 30 requests per minute per domain
        let throttle_manager = Arc::new(ThrottleManager::new(5, 30));
        throttle_manager.set_context(sync_ctx.clone());
        throttle_manager.set_git_naughty_list(git_naughty_list.clone());

        // Start the sync loop
        let _sync_loop_handle =
            purgatory
                .clone()
                .start_sync_loop(sync_ctx, throttle_manager, git_naughty_list.clone());
        info!("Purgatory sync loop started (1s interval)");

        // Setup shutdown handler for purgatory cleanup
        let shutdown_purgatory = purgatory.clone();
        let git_data_path = config.effective_git_data_path();

        // Start HTTP server with integrated relay and database
        info!("Starting HTTP server on {}", config.bind_address);

        // Run server until shutdown signal, then cleanup
        tokio::select! {
            result = http::run_server(
                config,
                relay_with_db.relay,
                relay_with_db.database,
                metrics,
                purgatory,
            ) => {
                result?
            }
            _ = signal::ctrl_c() => {
                info!("Received shutdown signal, cleaning up...");
            }
        }

        // Save purgatory state to disk
        let purgatory_save_path = PathBuf::from(&git_data_path).join("purgatory-state.json");
        if let Err(e) = shutdown_purgatory.save_to_disk(&purgatory_save_path) {
            error!("Failed to save purgatory state: {}", e);
        } else {
            info!("Purgatory state saved to disk");
        }

        // Save rejected events cache to disk
        let rejected_cache_path = PathBuf::from(&git_data_path).join("rejected-events-cache.json");
        if let Err(e) = shutdown_rejected_index.save_to_disk(&rejected_cache_path) {
            error!("Failed to save rejected events cache: {}", e);
        } else {
            info!("Rejected events cache saved to disk");
        }

        // Cleanup placeholder refs on shutdown
        let placeholder_ids = shutdown_purgatory.get_placeholder_event_ids();
        if !placeholder_ids.is_empty() {
            info!(
                "Cleaning up {} placeholder refs/nostr/ refs on shutdown",
                placeholder_ids.len()
            );
            git::cleanup_placeholder_refs(&git_data_path, &placeholder_ids);
        }
    }

    Ok(())
}
