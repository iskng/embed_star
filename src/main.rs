mod config;
mod embedder;
mod models;
mod pool;
mod surreal_client;

use crate::{
    config::Config,
    embedder::Embedder,
    models::Repo,
    pool::create_pool,
    surreal_client::SurrealClient,
};
use anyhow::Result;
use clap::Parser;
use std::{sync::Arc, time::Duration};
use tokio::{
    signal,
    sync::mpsc,
    time::{interval, sleep},
};
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            "warn,embed_star=info".into()
        }))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Arc::new(Config::parse());
    config.validate()?;

    info!("Starting embed_star");
    info!("{}", config);

    let pool = create_pool(config.clone()).await?;
    let client = Arc::new(SurrealClient::new(pool));
    let embedder = Arc::new(Embedder::new(config.clone())?);

    let total_repos = client.get_total_repos_count().await?;
    let embedded_repos = client.get_embedded_repos_count().await?;
    let pending_repos = client.get_pending_repos_count().await?;

    info!(
        "Database status: {} total repos, {} with embeddings, {} pending",
        total_repos, embedded_repos, pending_repos
    );

    let (tx, rx) = mpsc::channel::<Repo>(100);

    let client_clone = client.clone();
    let batch_processor = tokio::spawn(async move {
        process_batch_loop(rx, client_clone, embedder, config).await;
    });

    let client_clone = client.clone();
    let tx_clone = tx.clone();
    let initial_processor = tokio::spawn(async move {
        if let Err(e) = process_initial_batch(&client_clone, &tx_clone).await {
            error!("Error processing initial batch: {}", e);
        }
    });

    let live_query_processor = tokio::spawn(async move {
        if let Err(e) = process_live_query(client, tx).await {
            error!("Error in live query processor: {}", e);
        }
    });

    let stats_reporter = tokio::spawn(async move {
        report_stats_loop().await;
    });

    tokio::select! {
        _ = signal::ctrl_c() => {
            info!("Received shutdown signal");
        }
        _ = batch_processor => {
            warn!("Batch processor task ended unexpectedly");
        }
        _ = initial_processor => {
            info!("Initial batch processing completed");
        }
        _ = live_query_processor => {
            warn!("Live query processor ended unexpectedly");
        }
        _ = stats_reporter => {
            warn!("Stats reporter ended unexpectedly");
        }
    }

    info!("Shutting down embed_star");
    Ok(())
}

async fn process_initial_batch(
    client: &Arc<SurrealClient>,
    tx: &mpsc::Sender<Repo>,
) -> Result<()> {
    info!("Processing initial batch of repos needing embeddings");

    loop {
        let repos = client.get_repos_needing_embeddings(100).await?;
        if repos.is_empty() {
            info!("No more repos need embeddings in initial batch");
            break;
        }

        info!("Found {} repos needing embeddings", repos.len());
        for repo in repos {
            if tx.send(repo).await.is_err() {
                error!("Channel closed, stopping initial batch processing");
                break;
            }
        }

        sleep(Duration::from_millis(100)).await;
    }

    Ok(())
}

async fn process_live_query(
    client: Arc<SurrealClient>,
    tx: mpsc::Sender<Repo>,
) -> Result<()> {
    info!("Setting up live query for real-time updates");

    let mut rx = client.setup_live_query().await?;

    while let Some(repo) = rx.recv().await {
        info!("Live query: repo {} needs embedding", repo.full_name);
        if tx.send(repo).await.is_err() {
            error!("Channel closed, stopping live query processing");
            break;
        }
    }

    Ok(())
}

async fn process_batch_loop(
    mut rx: mpsc::Receiver<Repo>,
    client: Arc<SurrealClient>,
    embedder: Arc<Embedder>,
    config: Arc<Config>,
) {
    let mut batch = Vec::with_capacity(config.batch_size);
    let mut interval = interval(Duration::from_millis(config.batch_delay_ms));

    loop {
        tokio::select! {
            Some(repo) = rx.recv() => {
                batch.push(repo);
                if batch.len() >= config.batch_size {
                    process_batch(&batch, &client, &embedder).await;
                    batch.clear();
                }
            }
            _ = interval.tick() => {
                if !batch.is_empty() {
                    process_batch(&batch, &client, &embedder).await;
                    batch.clear();
                }
            }
        }
    }
}

async fn process_batch(
    batch: &[Repo],
    client: &Arc<SurrealClient>,
    embedder: &Arc<Embedder>,
) {
    info!("Processing batch of {} repos", batch.len());

    for repo in batch {
        let text = repo.prepare_text_for_embedding();
        
        match embedder.generate_embedding(&text).await {
            Ok(embedding) => {
                match client
                    .update_repo_embedding(&repo.id, embedding, embedder.model_name())
                    .await
                {
                    Ok(_) => {
                        info!(
                            "Successfully generated embedding for {}",
                            repo.full_name
                        );
                    }
                    Err(e) => {
                        error!(
                            "Failed to update embedding for {}: {}",
                            repo.full_name, e
                        );
                    }
                }
            }
            Err(e) => {
                error!(
                    "Failed to generate embedding for {}: {}",
                    repo.full_name, e
                );
            }
        }
    }
}

async fn report_stats_loop() {
    let mut interval = interval(Duration::from_secs(60));

    loop {
        interval.tick().await;
        info!("Statistics reporting would go here");
    }
}
