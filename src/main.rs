mod circuit_breaker;
mod cleanup;
mod config;
mod deduplication;
mod embedder;
mod error;
mod metrics;
mod migration;
mod models;
mod pool;
mod rate_limiter;
mod retry;
mod server;
mod shutdown;
mod surreal_client;
mod validation;

use crate::{
    circuit_breaker::{CircuitBreakerConfig, CircuitBreakerManager},
    cleanup::cleanup_locks_loop,
    config::Config,
    deduplication::{DeduplicationManager, LockGuard},
    embedder::Embedder,
    error::Result,
    metrics::Metrics,
    migration::run_migrations,
    models::Repo,
    pool::create_pool,
    rate_limiter::RateLimiterManager,
    retry::{with_retry, RetryConfig},
    server::{run_monitoring_server, AppState},
    shutdown::{setup_signal_handlers, GracefulShutdown, ShutdownController},
    surreal_client::SurrealClient,
    validation::{EmbeddingValidator, ValidationConfig},
};
use clap::Parser;
use prometheus::Registry;
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::mpsc,
    task::JoinHandle,
    time::{interval, sleep, Instant},
};
use tracing::{debug, error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use uuid::Uuid;

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

    let session_id = Uuid::new_v4();
    info!(session_id = %session_id, "Starting embed_star service");

    // Parse and validate configuration
    let config = Arc::new(Config::parse());
    config.validate()?;
    info!("Configuration loaded: {}", config);

    // Initialize metrics
    let registry = Arc::new(Registry::new());
    Metrics::register(&registry)?;
    info!("Metrics initialized");

    // Create database pool
    let pool = create_pool(config.clone()).await?;
    info!("Database connection pool created");

    // Run migrations
    run_migrations(&pool).await?;
    info!("Database migrations completed");

    // Initialize components
    let client = Arc::new(SurrealClient::new(pool.clone()));
    let embedder = Arc::new(Embedder::new(config.clone())?);
    let rate_limiter = Arc::new(RateLimiterManager::new());
    let deduplication = Arc::new(DeduplicationManager::new(pool.clone()));
    let circuit_breaker = Arc::new(CircuitBreakerManager::new());
    let validator = Arc::new(EmbeddingValidator::new(ValidationConfig::default()));

    // Configure circuit breakers for each provider
    match config.embedding_provider.as_str() {
        "openai" => {
            circuit_breaker.configure_service(
                "openai",
                CircuitBreakerConfig {
                    failure_threshold: 5,
                    timeout_duration: Duration::from_secs(120),
                    success_threshold: 3,
                    failure_rate_threshold: 0.5,
                    min_requests: 10,
                },
            );
        }
        "together" | "togetherai" => {
            circuit_breaker.configure_service(
                "together",
                CircuitBreakerConfig {
                    failure_threshold: 10,
                    timeout_duration: Duration::from_secs(60),
                    success_threshold: 5,
                    failure_rate_threshold: 0.6,
                    min_requests: 20,
                },
            );
        }
        "ollama" => {
            circuit_breaker.configure_service(
                "ollama",
                CircuitBreakerConfig {
                    failure_threshold: 3,
                    timeout_duration: Duration::from_secs(30),
                    success_threshold: 2,
                    failure_rate_threshold: 0.3,
                    min_requests: 5,
                },
            );
        }
        _ => {}
    }

    // Configure rate limits based on provider
    match config.embedding_provider.as_str() {
        "openai" => rate_limiter.configure_provider("openai", 3000).await?,
        "together" | "togetherai" => rate_limiter.configure_provider("together", 1000).await?,
        _ => {}
    }

    // Get initial statistics
    let total_repos = client.get_total_repos_count().await?;
    let embedded_repos = client.get_embedded_repos_count().await?;
    let pending_repos = client.get_pending_repos_count().await?;

    info!(
        total_repos = total_repos,
        embedded_repos = embedded_repos,
        pending_repos = pending_repos,
        "Database statistics"
    );
    
    metrics::set_pending_repos(pending_repos as i64);

    // Setup shutdown handling
    let shutdown_receiver = setup_signal_handlers().await;
    let (shutdown_controller, _) = ShutdownController::new();
    let mut graceful_shutdown = GracefulShutdown::new(shutdown_controller.clone());

    // Create processing channel
    let (tx, rx) = mpsc::channel::<Repo>(config.batch_size * 2);

    // Start monitoring server
    let monitoring_addr = format!("0.0.0.0:{}", config.monitoring_port.unwrap_or(9090));
    let app_state = AppState {
        db_pool: pool.clone(),
        registry: registry.clone(),
        deduplication: Some(deduplication.clone()),
    };
    
    let monitoring_handle: JoinHandle<()> = tokio::spawn({
        let mut shutdown_rx = shutdown_receiver.subscribe();
        async move {
            tokio::select! {
                result = run_monitoring_server(&monitoring_addr, app_state) => {
                    if let Err(e) = result {
                        error!("Monitoring server error: {}", e);
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Monitoring server shutting down");
                }
            }
        }
    });
    graceful_shutdown.register_task("monitoring_server".to_string(), monitoring_handle);

    // Start batch processor
    let batch_processor = tokio::spawn({
        let client = client.clone();
        let embedder = embedder.clone();
        let config = config.clone();
        let rate_limiter = rate_limiter.clone();
        let deduplication = deduplication.clone();
        let circuit_breaker = circuit_breaker.clone();
        let validator = validator.clone();
        let mut shutdown_rx = shutdown_receiver.subscribe();
        
        async move {
            process_batch_loop(
                rx,
                client,
                embedder,
                config,
                rate_limiter,
                deduplication,
                circuit_breaker,
                validator,
                shutdown_rx,
            ).await;
        }
    });
    graceful_shutdown.register_task("batch_processor".to_string(), batch_processor);

    // Start initial batch processor
    let initial_processor = tokio::spawn({
        let client = client.clone();
        let tx = tx.clone();
        let mut shutdown_rx = shutdown_receiver.subscribe();
        
        async move {
            if let Err(e) = process_initial_batch(&client, &tx, shutdown_rx).await {
                error!("Error processing initial batch: {}", e);
            }
        }
    });
    graceful_shutdown.register_task("initial_processor".to_string(), initial_processor);

    // Start live query processor
    let live_query_processor = tokio::spawn({
        let client = client.clone();
        let mut shutdown_rx = shutdown_receiver.subscribe();
        
        async move {
            if let Err(e) = process_live_query(client, tx, shutdown_rx).await {
                error!("Error in live query processor: {}", e);
            }
        }
    });
    graceful_shutdown.register_task("live_query_processor".to_string(), live_query_processor);

    // Start statistics reporter
    let stats_reporter = tokio::spawn({
        let client = client.clone();
        let mut shutdown_rx = shutdown_receiver.subscribe();
        
        async move {
            report_stats_loop(client, shutdown_rx).await;
        }
    });
    graceful_shutdown.register_task("stats_reporter".to_string(), stats_reporter);

    // Start lock cleanup task
    let lock_cleanup = tokio::spawn({
        let deduplication = deduplication.clone();
        let mut shutdown_rx = shutdown_receiver.subscribe();
        
        async move {
            cleanup_locks_loop(deduplication, shutdown_rx).await;
        }
    });
    graceful_shutdown.register_task("lock_cleanup".to_string(), lock_cleanup);

    // Wait for shutdown signal
    shutdown_receiver.wait_for_shutdown().await;
    
    // Perform graceful shutdown
    graceful_shutdown.shutdown(Duration::from_secs(30)).await;
    
    info!(session_id = %session_id, "embed_star service shut down successfully");
    Ok(())
}

async fn process_initial_batch(
    client: &Arc<SurrealClient>,
    tx: &mpsc::Sender<Repo>,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) -> Result<()> {
    info!("Starting initial batch processing");

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                info!("Initial batch processor received shutdown signal");
                break;
            }
            result = client.get_repos_needing_embeddings(100) => {
                match result {
                    Ok(repos) => {
                        if repos.is_empty() {
                            info!("No more repos need embeddings in initial batch");
                            break;
                        }

                        info!(count = repos.len(), "Found repos needing embeddings");
                        for repo in repos {
                            tokio::select! {
                                _ = shutdown_rx.recv() => {
                                    info!("Initial batch processor received shutdown signal");
                                    return Ok(());
                                }
                                send_result = tx.send(repo) => {
                                    if send_result.is_err() {
                                        error!("Channel closed, stopping initial batch processing");
                                        return Ok(());
                                    }
                                }
                            }
                        }

                        sleep(Duration::from_millis(100)).await;
                    }
                    Err(e) => {
                        error!("Error fetching repos: {}", e);
                        sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        }
    }

    Ok(())
}

async fn process_live_query(
    client: Arc<SurrealClient>,
    tx: mpsc::Sender<Repo>,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) -> Result<()> {
    info!("Starting live query processor");

    let mut rx = client.setup_live_query().await?;

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                info!("Live query processor received shutdown signal");
                break;
            }
            repo_opt = rx.recv() => {
                match repo_opt {
                    Some(repo) => {
                        info!(repo = %repo.full_name, "Live query: repo needs embedding");
                        if tx.send(repo).await.is_err() {
                            error!("Channel closed, stopping live query processing");
                            break;
                        }
                    }
                    None => {
                        warn!("Live query channel closed");
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

async fn process_batch_loop(
    mut rx: mpsc::Receiver<Repo>,
    client: Arc<SurrealClient>,
    embedder: Arc<Embedder>,
    config: Arc<Config>,
    rate_limiter: Arc<RateLimiterManager>,
    deduplication: Arc<DeduplicationManager>,
    circuit_breaker: Arc<CircuitBreakerManager>,
    validator: Arc<EmbeddingValidator>,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) {
    let mut batch = Vec::with_capacity(config.batch_size);
    let mut interval = interval(Duration::from_millis(config.batch_delay_ms));
    let retry_config = RetryConfig::default();

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                info!("Batch processor received shutdown signal");
                if !batch.is_empty() {
                    info!("Processing final batch of {} repos", batch.len());
                    process_batch(&batch, &client, &embedder, &rate_limiter, &deduplication, &circuit_breaker, &validator, &retry_config).await;
                }
                break;
            }
            Some(repo) = rx.recv() => {
                batch.push(repo);
                if batch.len() >= config.batch_size {
                    process_batch(&batch, &client, &embedder, &rate_limiter, &deduplication, &circuit_breaker, &validator, &retry_config).await;
                    batch.clear();
                }
            }
            _ = interval.tick() => {
                if !batch.is_empty() {
                    process_batch(&batch, &client, &embedder, &rate_limiter, &deduplication, &circuit_breaker, &validator, &retry_config).await;
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
    rate_limiter: &Arc<RateLimiterManager>,
    deduplication: &Arc<DeduplicationManager>,
    circuit_breaker: &Arc<CircuitBreakerManager>,
    validator: &Arc<EmbeddingValidator>,
    retry_config: &RetryConfig,
) {
    let batch_id = Uuid::new_v4();
    info!(
        batch_id = %batch_id,
        batch_size = batch.len(),
        "Processing batch"
    );

    for repo in batch {
        let span = tracing::info_span!(
            "process_repo",
            repo_id = %repo.id,
            repo_name = %repo.full_name,
            batch_id = %batch_id
        );
        let _enter = span.enter();

        // Try to acquire lock for this repository
        match deduplication.try_acquire_lock(&repo.id).await {
            Ok(true) => {
                // Lock acquired, proceed with processing
                debug!("Acquired lock for repo {}", repo.full_name);
            }
            Ok(false) => {
                // Another instance is processing this repo
                debug!("Repo {} is being processed by another instance", repo.full_name);
                continue;
            }
            Err(e) => {
                error!("Failed to check lock for repo {}: {}", repo.full_name, e);
                continue;
            }
        }

        // Create lock guard for automatic cleanup
        let lock_guard = LockGuard::new(deduplication, repo.id.clone());

        let text = repo.prepare_text_for_embedding();
        let provider = embedder.model_name();
        
        // Check rate limit
        if let Err(e) = rate_limiter.wait_for_permit(provider).await {
            error!("Rate limit error: {}", e);
            continue;
        }
        
        let start = Instant::now();
        
        let embedding_result = with_circuit_breaker!(
            circuit_breaker,
            provider,
            with_retry(
                &format!("generate_embedding_{}", repo.full_name),
                retry_config,
                || async {
                    embedder.generate_embedding(&text).await
                        .map_err(|e| error::EmbedError::EmbeddingProvider(e.to_string()))
                },
            ).await
        );
        
        match embedding_result {
            Ok(mut embedding) => {
                let duration = start.elapsed().as_secs_f64();
                
                // Validate the embedding
                match validator.validate(&embedding, &repo.full_name) {
                    Ok(_) => {
                        metrics::record_embedding_generated(provider, embedder.model_name(), duration);
                        metrics::record_provider_request(provider, true);
                    }
                    Err(e) => {
                        error!("Embedding validation failed for {}: {}", repo.full_name, e);
                        metrics::record_embedding_error(provider, "validation_failed");
                        // Release lock with failure status
                        if let Err(e) = lock_guard.release("failed").await {
                            error!("Failed to release lock: {}", e);
                        }
                        continue;
                    }
                }
                
                let update_result = with_retry(
                    &format!("update_embedding_{}", repo.full_name),
                    retry_config,
                    || async {
                        client
                            .update_repo_embedding(&repo.id, embedding.clone(), embedder.model_name())
                            .await
                    },
                ).await;
                
                match update_result {
                    Ok(_) => {
                        info!(
                            duration_ms = duration * 1000.0,
                            embedding_size = embedding.len(),
                            "Successfully generated embedding"
                        );
                        // Release lock with success status
                        if let Err(e) = lock_guard.release("completed").await {
                            error!("Failed to release lock: {}", e);
                        }
                    }
                    Err(e) => {
                        error!("Failed to update embedding: {}", e);
                        metrics::record_embedding_error(provider, e.error_code());
                        // Release lock with failure status
                        if let Err(e) = lock_guard.release("failed").await {
                            error!("Failed to release lock: {}", e);
                        }
                    }
                }
            }
            Err(e) => {
                error!("Failed to generate embedding: {}", e);
                metrics::record_embedding_error(provider, e.error_code());
                metrics::record_provider_request(provider, false);
                // Release lock with failure status
                if let Err(e) = lock_guard.release("failed").await {
                    error!("Failed to release lock: {}", e);
                }
            }
        }
    }
}

async fn report_stats_loop(
    client: Arc<SurrealClient>,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) {
    let mut interval = interval(Duration::from_secs(60));

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                info!("Stats reporter received shutdown signal");
                break;
            }
            _ = interval.tick() => {
                match client.get_pending_repos_count().await {
                    Ok(count) => {
                        metrics::set_pending_repos(count as i64);
                        info!(
                            pending_repos = count,
                            "Updated statistics"
                        );
                    }
                    Err(e) => {
                        error!("Failed to get pending repos count: {}", e);
                    }
                }
            }
        }
    }
}