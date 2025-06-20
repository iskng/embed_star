use crate::{
    circuit_breaker::{CircuitBreakerConfig, CircuitBreakerManager},
    config::Config,
    embedder::Embedder,
    embedding_cache::{cache_cleanup_task, EmbeddingCache},
    error::Result,
    metrics::Metrics,
    migration::run_migrations,
    models::Repo,
    pool::create_pool,
    pool_metrics::monitor_pool_metrics,
    process_batch::process_batch,
    rate_limiter::RateLimiterManager,
    retry::RetryConfig,
    server::{run_monitoring_server, AppState},
    shutdown::{setup_signal_handlers, GracefulShutdown, ShutdownController},
    surreal_client::SurrealClient,
    validation::{EmbeddingValidator, ValidationConfig},
};
use prometheus::Registry;
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::mpsc,
    task::JoinHandle,
    time::{interval, sleep},
};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Run the embed_star service with the given configuration
pub async fn run_with_config(config: Config) -> anyhow::Result<()> {
    let session_id = Uuid::new_v4();
    info!(session_id = %session_id, "Starting embed_star service");

    // Validate configuration
    let config = Arc::new(config);
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
    let circuit_breaker = Arc::new(CircuitBreakerManager::new());
    let validator = Arc::new(EmbeddingValidator::new(ValidationConfig::default()));
    let cache = Arc::new(EmbeddingCache::new(10_000, 3600)); // 10k entries, 1 hour TTL

    // Configure circuit breakers for each provider
    match config.embedding_provider.as_str() {
        "openai" => {
            rate_limiter.configure_provider("openai", 3000).await?;
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
        "together" => {
            rate_limiter.configure_provider("together", 1000).await?;
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
    
    crate::metrics::set_pending_repos(pending_repos as i64);

    // Setup shutdown handling
    let shutdown_receiver = setup_signal_handlers().await;
    let (shutdown_controller, _) = ShutdownController::new();
    let mut graceful_shutdown = GracefulShutdown::new(shutdown_controller.clone());

    // Create processing channel with larger buffer for parallel workers
    let (tx, rx) = mpsc::channel::<Repo>(config.batch_size * config.parallel_workers * 2);

    // Start monitoring server
    let monitoring_addr = format!("0.0.0.0:{}", config.monitoring_port.unwrap_or(9090));
    let app_state = AppState {
        db_pool: pool.clone(),
        registry: registry.clone(),
        embedder: embedder.clone(),
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

    // Create shared receiver wrapped in Arc<Mutex> for multiple workers
    let rx = Arc::new(tokio::sync::Mutex::new(rx));

    // Start multiple batch processor workers
    for worker_id in 0..config.parallel_workers {
        let batch_processor = tokio::spawn({
            let rx = rx.clone();
            let client = client.clone();
            let embedder = embedder.clone();
            let config = config.clone();
            let rate_limiter = rate_limiter.clone();
            let circuit_breaker = circuit_breaker.clone();
            let validator = validator.clone();
            let cache = cache.clone();
            let shutdown_rx = shutdown_receiver.subscribe();
            
            async move {
                info!("Starting batch processor worker {}", worker_id);
                process_batch_loop_worker(
                    worker_id,
                    rx,
                    client,
                    embedder,
                    config,
                    rate_limiter,
                    circuit_breaker,
                    validator,
                    cache,
                    shutdown_rx,
                ).await;
            }
        });
        graceful_shutdown.register_task(
            format!("batch_processor_{}", worker_id),
            batch_processor,
        );
    }

    // Start initial batch processor
    let initial_processor = tokio::spawn({
        let client = client.clone();
        let tx = tx.clone();
        let shutdown_rx = shutdown_receiver.subscribe();
        
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
        let shutdown_rx = shutdown_receiver.subscribe();
        
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
        let shutdown_rx = shutdown_receiver.subscribe();
        
        async move {
            report_stats_loop(client, shutdown_rx).await;
        }
    });
    graceful_shutdown.register_task("stats_reporter".to_string(), stats_reporter);


    // Start pool metrics monitor
    let pool_monitor = tokio::spawn({
        let pool = pool.clone();
        let shutdown_rx = shutdown_receiver.subscribe();
        
        async move {
            monitor_pool_metrics(pool, shutdown_rx).await;
        }
    });
    graceful_shutdown.register_task("pool_monitor".to_string(), pool_monitor);

    // Start cache cleanup task
    let cache_cleanup = tokio::spawn({
        let cache = cache.clone();
        let shutdown_rx = shutdown_receiver.subscribe();
        
        async move {
            cache_cleanup_task(cache, shutdown_rx).await;
        }
    });
    graceful_shutdown.register_task("cache_cleanup".to_string(), cache_cleanup);

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

async fn process_batch_loop_worker(
    worker_id: usize,
    rx: Arc<tokio::sync::Mutex<mpsc::Receiver<Repo>>>,
    client: Arc<SurrealClient>,
    embedder: Arc<Embedder>,
    config: Arc<Config>,
    rate_limiter: Arc<RateLimiterManager>,
    circuit_breaker: Arc<CircuitBreakerManager>,
    validator: Arc<EmbeddingValidator>,
    cache: Arc<EmbeddingCache>,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) {
    let mut batch = Vec::with_capacity(config.batch_size);
    let mut interval = interval(Duration::from_millis(config.batch_delay_ms));
    let retry_config = RetryConfig::default();

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                info!("Worker {} received shutdown signal", worker_id);
                if !batch.is_empty() {
                    info!("Worker {} processing final batch of {} repos", worker_id, batch.len());
                    process_batch(&batch, &client, &embedder, &rate_limiter, &circuit_breaker, &validator, &cache, &retry_config).await;
                }
                break;
            }
            _ = interval.tick() => {
                // Try to fill the batch
                let mut rx_guard = rx.lock().await;
                while batch.len() < config.batch_size {
                    match rx_guard.try_recv() {
                        Ok(repo) => batch.push(repo),
                        Err(_) => break,
                    }
                }
                drop(rx_guard);

                if !batch.is_empty() {
                    debug!("Worker {} processing batch of {} repos", worker_id, batch.len());
                    process_batch(&batch, &client, &embedder, &rate_limiter, &circuit_breaker, &validator, &cache, &retry_config).await;
                    batch.clear();
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
                        crate::metrics::set_pending_repos(count as i64);
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