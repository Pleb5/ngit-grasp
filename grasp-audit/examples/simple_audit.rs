//! Simple audit example
//!
//! Run with: cargo run --example simple_audit

use grasp_audit::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Create audit config for CI testing
    let config = AuditConfig::ci();
    
    println!("GRASP Audit Example");
    println!("==================");
    println!("Audit Run ID: {}", config.run_id);
    println!();
    
    // Connect to relay
    println!("Connecting to relay at ws://localhost:7000...");
    let client = match AuditClient::new("ws://localhost:7000", config).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect: {}", e);
            eprintln!();
            eprintln!("Make sure a Nostr relay is running at ws://localhost:7000");
            eprintln!("You can use: https://github.com/rust-nostr/nostr/tree/master/crates/nostr-relay-builder");
            return Err(e);
        }
    };
    
    if !client.is_connected().await {
        eprintln!("Not connected to relay");
        return Err(anyhow!("Connection failed"));
    }
    
    println!("✓ Connected");
    println!();
    
    // Run NIP-01 smoke tests
    println!("Running NIP-01 smoke tests...");
    println!();
    
    let results = specs::Nip01SmokeTests::run_all(&client).await;
    
    // Print results
    results.print_report();
    
    // Exit with error if tests failed
    if !results.all_passed() {
        std::process::exit(1);
    }
    
    Ok(())
}
