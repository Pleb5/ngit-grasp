//! GRASP Audit CLI Tool

use clap::{Parser, Subcommand};
use grasp_audit::*;

#[derive(Parser)]
#[command(name = "grasp-audit")]
#[command(about = "GRASP audit and compliance testing tool", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run audit tests against a server
    Audit {
        /// Relay URL (e.g., ws://localhost:7000)
        #[arg(short, long)]
        relay: String,
        
        /// Mode: ci or production
        #[arg(short, long, default_value = "ci")]
        mode: String,
        
        /// Spec to test (nip01-smoke, all)
        #[arg(short, long, default_value = "nip01-smoke")]
        spec: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into())
        )
        .init();
    
    let cli = Cli::parse();
    
    match cli.command {
        Commands::Audit { relay, mode, spec } => {
            let config = match mode.as_str() {
                "ci" => AuditConfig::ci(),
                "production" => AuditConfig::production(),
                _ => return Err(anyhow!("Invalid mode: {}. Use 'ci' or 'production'", mode)),
            };
            
            println!("🔍 GRASP Audit Tool");
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!("Relay:   {}", relay);
            println!("Mode:    {}", mode);
            println!("Spec:    {}", spec);
            println!("Run ID:  {}", config.run_id);
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!();
            
            println!("Connecting to relay...");
            let client = AuditClient::new(&relay, config).await
                .map_err(|e| anyhow!("Failed to connect to relay: {}", e))?;
            
            if !client.is_connected().await {
                return Err(anyhow!("Could not establish connection to relay"));
            }
            
            println!("✓ Connected\n");
            
            let results = match spec.as_str() {
                "nip01-smoke" => {
                    println!("Running NIP-01 smoke tests...\n");
                    specs::Nip01SmokeTests::run_all(&client).await
                }
                "all" => {
                    println!("Running all tests...\n");
                    specs::Nip01SmokeTests::run_all(&client).await
                }
                _ => return Err(anyhow!("Unknown spec: {}. Use 'nip01-smoke' or 'all'", spec)),
            };
            
            results.print_report();
            
            if !results.all_passed() {
                println!("❌ Some tests failed");
                std::process::exit(1);
            } else {
                println!("✅ All tests passed!");
            }
        }
    }
    
    Ok(())
}
