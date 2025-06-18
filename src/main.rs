mod circuit_breaker;
mod cleanup;
mod config;
mod deduplication;
mod embedder;
mod embedding_cache;
mod error;
mod metrics;
mod migration;
mod models;
mod pool;
mod pool_metrics;
mod process_batch;
mod rate_limiter;
mod retry;
mod server;
mod service;
mod shutdown;
mod surreal_client;
mod validation;

use config::Config;
use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();

    // Initialize structured logging with correlation IDs
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            "warn,embed_star=info,tower_http=debug".into()
        }))
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .compact()
        )
        .init();

    // Parse configuration and run service
    let config = Config::parse();
    service::run_with_config(config).await
}