//! GRASP Audit CLI Tool

use clap::{Parser, Subcommand};
use grasp_audit::*;
use std::path::PathBuf;

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

        /// Spec to test (nip01-smoke, nip11, event-acceptance, cors, git-clone, push-auth, repo-creation, all)
        #[arg(short, long, default_value = "all")]
        spec: String,

        /// Git data directory (required for cors, git-clone, push-auth, repo-creation specs)
        #[arg(short, long)]
        git_data_dir: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Audit { relay, mode, spec, git_data_dir } => {

            let mut config = match mode.as_str() {
                "ci" => AuditConfig::ci(),
                "production" => AuditConfig::production(),
                _ => return Err(anyhow!("Invalid mode: {}. Use 'ci' or 'production'", mode)),
            };
            
            // Audit needs to create events to test the relay, so disable read-only mode
            config.read_only = false;

            // Derive relay_domain from relay URL (e.g., "ws://localhost:8081" -> "localhost:8081")
            let relay_domain = relay
                .replace("ws://", "")
                .replace("wss://", "")
                .trim_end_matches('/')
                .to_string();

            println!("🔍 GRASP Audit Tool");
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!("Relay:   {}", relay);
            println!("Mode:    {}", mode);
            println!("Spec:    {}", spec);
            println!("Run ID:  {}", config.run_id);
            if let Some(ref dir) = git_data_dir {
                println!("Git Dir: {}", dir.display());
            }
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!();

            println!("Connecting to relay...");
            let client = AuditClient::new(&relay, config)
                .await
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
                "nip11" => {
                    println!("Running NIP-11 document tests...\n");
                    specs::Nip11DocumentTests::run_all(&client).await
                }
                "event-acceptance" => {
                    println!("Running event acceptance policy tests...\n");
                    specs::EventAcceptancePolicyTests::run_all(&client).await
                }
                "cors" => {
                    println!("Running CORS tests...\n");
                    specs::CorsTests::run_all(&client, &relay_domain).await
                }
                "git-clone" => {
                    println!("Running Git clone tests...\n");
                    specs::GitCloneTests::run_all(&client, &relay_domain).await
                }
                "push-auth" => {
                    println!("Running push authorization tests...\n");
                    specs::PushAuthorizationTests::run_all(&client, &relay_domain).await
                }
                "repo-creation" => {
                    println!("Running repository creation tests...\n");
                    specs::RepositoryCreationTests::run_all(&client, &relay_domain).await
                }
                "all" => {
                    println!("Running all tests...\n");
                    let mut all_results = AuditResult::new("All GRASP-01 Tests");

                    // Repository creation tests
                    println!("  → Repository creation tests...");
                    let repo_results = specs::RepositoryCreationTests::run_all(&client, &relay_domain).await;
                    all_results.merge(repo_results);

                    // Git clone tests
                    println!("  → Git clone tests...");
                    let clone_results = specs::GitCloneTests::run_all(&client, &relay_domain).await;
                    all_results.merge(clone_results);

                    // Push authorization tests
                    println!("  → Push authorization tests...");
                    let push_results = specs::PushAuthorizationTests::run_all(&client, &relay_domain).await;
                    all_results.merge(push_results);

                    // Event acceptance policy tests
                    println!("  → Event acceptance policy tests...");
                    let event_results = specs::EventAcceptancePolicyTests::run_all(&client).await;
                    all_results.merge(event_results);

                    // NIP-01 smoke tests
                    println!("  → NIP-01 smoke tests...");
                    let nip01_results = specs::Nip01SmokeTests::run_all(&client).await;
                    all_results.merge(nip01_results);
                    
                    // NIP-11 document tests
                    println!("  → NIP-11 document tests...");
                    let nip11_results = specs::Nip11DocumentTests::run_all(&client).await;
                    all_results.merge(nip11_results);
                    
                    // CORS tests
                    println!("  → CORS tests...");
                    let cors_results = specs::CorsTests::run_all(&client, &relay_domain).await;
                    all_results.merge(cors_results);
                    
                    println!();
                    all_results
                }
                _ => {
                    return Err(anyhow!(
                        "Unknown spec: {}. Use 'nip01-smoke', 'nip11', 'event-acceptance', 'cors', 'git-clone', 'push-auth', 'repo-creation', or 'all'",
                        spec
                    ))
                }
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
