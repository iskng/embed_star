pub mod circuit_breaker;
pub mod config;
pub mod embedder;
pub mod embedding_cache;
pub mod embedding_validation;
pub mod error;
pub mod metrics;
pub mod migration;
pub mod models;
pub mod pool;
pub mod pool_metrics;
pub mod process_batch;
pub mod rate_limiter;
pub mod retry;
pub mod server;
pub mod service;
pub mod shutdown;
pub mod surreal_client;
pub mod validation;

use clap::Parser;

/// Run the embed_star service
pub async fn run_service() -> anyhow::Result<()> {
    // Parse config from environment/CLI
    let config = config::Config::parse();
    
    // Run the service
    service::run_with_config(config).await
}