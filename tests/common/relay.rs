//! Test relay fixture
//!
//! Provides automatic relay lifecycle management for integration tests.

use nostr_sdk::ToBech32;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::time::sleep;

/// Test relay fixture that manages relay lifecycle
///
/// Automatically starts and stops the ngit-grasp relay for testing.
/// Uses a random port to avoid conflicts and cleans up created repositories.
pub struct TestRelay {
    process: Child,
    url: String,
    port: u16,
}

impl TestRelay {
    /// Start a test relay instance
    ///
    /// # Example
    ///
    /// ```no_run
    /// use common::TestRelay;
    ///
    /// #[tokio::test]
    /// async fn test_something() {
    ///     let relay = TestRelay::start().await;
    ///     // Use relay.url() for testing
    ///     relay.stop().await;
    /// }
    /// ```
    pub async fn start() -> Self {
        Self::start_with_options(Self::find_free_port(), None).await
    }

    /// Start relay on a specific port
    pub async fn start_with_port(port: u16) -> Self {
        Self::start_with_options(port, None).await
    }

    /// Start relay with sync from another relay
    ///
    /// # Example
    ///
    /// ```no_run
    /// use common::TestRelay;
    ///
    /// #[tokio::test]
    /// async fn test_sync() {
    ///     let source = TestRelay::start().await;
    ///     let syncing = TestRelay::start_with_sync(source.url()).await;
    ///     // ... test sync behavior ...
    ///     syncing.stop().await;
    ///     source.stop().await;
    /// }
    /// ```
    pub async fn start_with_sync(sync_relay_url: &str) -> Self {
        Self::start_with_options(Self::find_free_port(), Some(sync_relay_url.to_string())).await
    }

    /// Start relay with options
    async fn start_with_options(port: u16, sync_relay_url: Option<String>) -> Self {
        let bind_address = format!("127.0.0.1:{}", port);
        let url = format!("ws://127.0.0.1:{}", port);

        // Create temporary directory for git repositories
        let git_data_dir =
            tempfile::tempdir().expect("Failed to create temporary git data directory");

        // Use the built binary directly (faster than cargo run)
        let binary_path = std::env::current_exe()
            .expect("Failed to get current exe")
            .parent()
            .expect("Failed to get parent dir")
            .parent()
            .expect("Failed to get grandparent dir")
            .join("ngit-grasp");

        // Generate a test owner npub (using a random keypair)
        let test_keys = nostr_sdk::Keys::generate();
        let test_npub = test_keys
            .public_key()
            .to_bech32()
            .expect("Failed to generate test npub");

        // Start the relay process
        let mut cmd = Command::new(&binary_path);
        cmd.env("NGIT_BIND_ADDRESS", &bind_address)
            .env("NGIT_DOMAIN", &bind_address) // Set domain to match bind address
            .env("NGIT_GIT_DATA_PATH", git_data_dir.path())
            .env("NGIT_OWNER_NPUB", &test_npub)
            .env("RUST_LOG", "warn") // Less logging during tests
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        // Add sync relay URL if provided
        if let Some(ref sync_url) = sync_relay_url {
            cmd.env("NGIT_SYNC_RELAY_URL", sync_url);
        }

        let process = cmd.spawn().expect("Failed to start relay process");

        let relay = Self { process, url, port };

        // Wait for relay to be ready
        relay.wait_for_ready().await;

        relay
    }

    /// Get the relay WebSocket URL
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Get the relay domain (host:port)
    pub fn domain(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    /// Wait for the relay to be ready to accept connections
    async fn wait_for_ready(&self) {
        let max_attempts = 50; // 5 seconds total
        let delay = Duration::from_millis(100);

        for attempt in 0..max_attempts {
            // Try to connect to the relay
            match tokio::net::TcpStream::connect(format!("127.0.0.1:{}", self.port)).await {
                Ok(_) => {
                    // Connection successful, relay is ready
                    // Give it a tiny bit more time to fully initialize
                    sleep(Duration::from_millis(100)).await;
                    return;
                }
                Err(_) => {
                    if attempt == max_attempts - 1 {
                        panic!("Relay failed to start after {} attempts", max_attempts);
                    }
                    sleep(delay).await;
                }
            }
        }
    }

    /// Stop the relay
    pub async fn stop(mut self) {
        // Kill the process (gracefully if possible)
        let _ = self.process.kill();

        // Wait a bit for graceful shutdown
        sleep(Duration::from_millis(100)).await;

        // Force kill if still running
        let _ = self.process.kill();
        let _ = self.process.wait();
    }

    /// Find a free port to use for testing
    fn find_free_port() -> u16 {
        use std::net::TcpListener;

        // Bind to port 0 to get a random free port
        let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind to random port");

        let port = listener
            .local_addr()
            .expect("Failed to get local address")
            .port();

        // Drop the listener to free the port
        drop(listener);

        port
    }
}

impl Drop for TestRelay {
    fn drop(&mut self) {
        // Ensure process is killed when TestRelay is dropped
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_free_port() {
        let port = TestRelay::find_free_port();
        assert!(port > 0);
        // Port is u16, so it's always < 65536
    }
}
