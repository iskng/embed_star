use embed_star::config::Config;
use embed_star::service::run_with_config;
use clap::Parser;
use std::env;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging with clean format
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,embed_star=debug"));
    
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer()
            .with_target(false)
            .with_thread_ids(false)
            .with_level(true)
            .compact())
        .init();

    // Set up environment
    env::set_var("DB_URL", "ws://localhost:8000");
    env::set_var("DB_USERNAME", "root");
    env::set_var("DB_PASSWORD", "root");
    env::set_var("DB_NAMESPACE", "test");
    env::set_var("DB_DATABASE", "test");
    env::set_var("EMBEDDING_PROVIDER", "ollama");
    env::set_var("EMBEDDING_MODEL", "nomic-embed-text");
    env::set_var("OLLAMA_BASE_URL", "http://localhost:11434");
    env::set_var("BATCH_SIZE", "2");
    env::set_var("BATCH_DELAY_MS", "1000");
    env::set_var("PARALLEL_WORKERS", "1");

    // Parse config from environment variables (clap will read from env due to #[arg(env = ...)])
    let config = Config::parse();
    
    println!("\n=== Testing Clean Logging Output ===\n");
    
    // Run for a short time to see logging
    tokio::select! {
        result = run_with_config(config) => {
            result?;
        }
        _ = tokio::time::sleep(tokio::time::Duration::from_secs(10)) => {
            println!("\n=== Test completed ===");
        }
    }

    Ok(())
}