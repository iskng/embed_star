use crate::{
    circuit_breaker::CircuitBreakerManager,
    embedder::Embedder,
    embedding_cache::EmbeddingCache,
    error::EmbedError,
    metrics,
    models::Repo,
    rate_limiter::RateLimiterManager,
    retry::{with_retry, RetryConfig},
    surreal_client::{EmbeddingUpdate, SurrealClient},
    validation::EmbeddingValidator,
    with_circuit_breaker,
};
use std::sync::Arc;
use tokio::time::Instant;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

pub async fn process_batch(
    batch: &[Repo],
    client: &Arc<SurrealClient>,
    embedder: &Arc<Embedder>,
    rate_limiter: &Arc<RateLimiterManager>,
    circuit_breaker: &Arc<CircuitBreakerManager>,
    validator: &Arc<EmbeddingValidator>,
    cache: &Arc<EmbeddingCache>,
    retry_config: &RetryConfig,
) {
    let batch_id = Uuid::new_v4();
    let batch_size = batch.len();
    
    // Create a cleaner log with just the essential info
    info!(
        batch_id = %batch_id,
        batch_size = batch_size,
        "Processing batch"
    );

    // Log the repos being processed in this batch (at debug level)
    debug!(
        batch_id = %batch_id,
        repos = ?batch.iter().map(|r| &r.full_name).collect::<Vec<_>>(),
        "Batch contains repos"
    );

    // Collect successful updates for batch processing
    let mut pending_updates = Vec::new();

    for (idx, repo) in batch.iter().enumerate() {
        // Process each repo with a clean span
        let repo_span = tracing::debug_span!(
            "process_repo",
            repo_name = %repo.full_name,
            repo_index = idx,
            batch_progress = %format!("{}/{}", idx + 1, batch_size)
        );
        
        let _enter = repo_span.enter();
        
        debug!("Processing repository");

        let text = repo.prepare_text_for_embedding();
        let provider = embedder.model_name();
        let cache_key = EmbeddingCache::cache_key(&repo.full_name, provider);
        
        // Check cache first
        if let Some((cached_embedding, _cached_model)) = cache.get(&cache_key) {
            info!("Using cached embedding");
            
            // Add to pending updates with cached embedding
            pending_updates.push(EmbeddingUpdate {
                repo_id: repo.id.clone(),
                embedding: cached_embedding,
            });
            continue;
        }

        // Wait for rate limit permit
        if let Err(e) = rate_limiter.wait_for_permit(&provider).await {
            error!(error = %e, "Rate limit error, skipping repo");
            metrics::record_rate_limit(&provider);
            continue;
        }

        // Generate embedding with circuit breaker
        let start = Instant::now();
        
        let embedding_result = with_circuit_breaker!(
            circuit_breaker,
            provider,
            with_retry(
                &format!("generate_embedding_{}", repo.full_name),
                retry_config,
                || async {
                    embedder.generate_embedding(&text).await
                        .map_err(|e| EmbedError::EmbeddingProvider(e.to_string()))
                },
            ).await
        );
        
        match embedding_result {
            Ok(embedding) => {
                let duration = start.elapsed().as_secs_f64();
                
                // Validate the embedding
                match validator.validate(&embedding, &repo.full_name) {
                    Ok(_) => {
                        metrics::record_embedding_generated(provider, embedder.model_name(), duration);
                        metrics::record_provider_request(provider, true);
                        
                        // Cache the embedding
                        cache.put(
                            cache_key,
                            embedding.clone(),
                            embedder.model_name().to_string(),
                        );
                        
                        // Add to pending updates
                        pending_updates.push(EmbeddingUpdate {
                            repo_id: repo.id.clone(),
                            embedding,
                        });
                        
                        info!(
                            duration_ms = (duration * 1000.0) as u64,
                            "Generated embedding successfully"
                        );
                    }
                    Err(e) => {
                        error!(error = %e, "Embedding validation failed");
                        metrics::record_provider_request(provider, false);
                    }
                }
            }
            Err(e) => {
                error!(error = %e, "Failed to generate embedding");
                metrics::record_embedding_error(provider, e.error_code());
                metrics::record_provider_request(provider, false);
            }
        }
    }

    // Batch update embeddings if any were generated
    if !pending_updates.is_empty() {
        let update_count = pending_updates.len();
        match client.batch_update_embeddings(pending_updates).await {
            Ok(result) => {
                info!(
                    batch_id = %batch_id,
                    successful = result.successful,
                    failed = result.failed,
                    duration_ms = result.duration.as_millis(),
                    "Batch update completed"
                );
            }
            Err(e) => {
                error!(
                    batch_id = %batch_id,
                    updates_lost = update_count,
                    error = %e,
                    "Failed to batch update embeddings"
                );
            }
        }
    } else {
        warn!(
            batch_id = %batch_id,
            "No embeddings were generated in this batch"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        embedder::Embedder,
        models::{Repo, RepoOwner},
        pool::create_pool,
        surreal_client::SurrealClient,
        validation::ValidationConfig,
    };
    use chrono::Utc;
    use surrealdb::RecordId;
    use std::sync::Arc;

    async fn setup_test_environment() -> (
        Arc<SurrealClient>,
        Arc<Embedder>,
        Arc<RateLimiterManager>,
        Arc<CircuitBreakerManager>,
        Arc<EmbeddingValidator>,
        Arc<EmbeddingCache>,
        RetryConfig,
    ) {
        let config = Arc::new(Config {
            db_url: "memory://test".to_string(),
            db_user: "root".to_string(),
            db_pass: "root".to_string(),
            db_namespace: "test_ns".to_string(),
            db_database: "test_db".to_string(),
            embedding_provider: "ollama".to_string(),
            ollama_url: "http://localhost:11434".to_string(),
            openai_api_key: None,
            together_api_key: None,
            embedding_model: "test-model".to_string(),
            batch_size: 10,
            pool_size: 2,
            retry_attempts: 1,
            retry_delay_ms: 10,
            batch_delay_ms: 100,
            monitoring_port: Some(9090),
            parallel_workers: 1,
            token_limit: 8000,
            pool_max_size: 5,
            pool_timeout_secs: 30,
            pool_wait_timeout_secs: 10,
            pool_create_timeout_secs: 30,
            pool_recycle_timeout_secs: 30,
        });

        let pool = create_pool(config.clone()).await.expect("Failed to create pool");
        let conn = pool.get().await.expect("Failed to get connection");
        conn.query("DEFINE TABLE repo SCHEMALESS").await.expect("Failed to create table");

        let client = Arc::new(SurrealClient::new(pool));
        let embedder = Arc::new(Embedder::new(config.clone()).expect("Failed to create embedder"));
        let rate_limiter = Arc::new(RateLimiterManager::new());
        let circuit_breaker = Arc::new(CircuitBreakerManager::new());
        let validator = Arc::new(EmbeddingValidator::new(ValidationConfig::default()));
        let cache = Arc::new(EmbeddingCache::new(100, 3600));
        let retry_config = RetryConfig::default();

        (client, embedder, rate_limiter, circuit_breaker, validator, cache, retry_config)
    }

    fn create_test_repo(id: &str) -> Repo {
        let now = Utc::now();
        Repo {
            id: RecordId::from(("repo", id)),
            github_id: 123456,
            name: format!("test-{}", id),
            full_name: format!("owner/test-{}", id),
            description: Some("Test repository for batch processing".to_string()),
            url: format!("https://github.com/owner/test-{}", id),
            stars: 42,
            language: Some("Rust".to_string()),
            owner: RepoOwner {
                login: "owner".to_string(),
                avatar_url: "https://github.com/owner.png".to_string(),
            },
            is_private: false,
            created_at: now,
            updated_at: now,
            embedding: None,
            embedding_generated_at: None,
        }
    }

    #[tokio::test]
    async fn test_process_empty_batch() {
        let (client, embedder, rate_limiter, circuit_breaker, validator, cache, retry_config) = 
            setup_test_environment().await;

        let batch: Vec<Repo> = vec![];
        
        // Should complete without errors
        process_batch(
            &batch,
            &client,
            &embedder,
            &rate_limiter,
            &circuit_breaker,
            &validator,
            &cache,
            &retry_config,
        ).await;
    }

    #[tokio::test] 
    async fn test_process_batch_with_cache_hit() {
        let (client, embedder, rate_limiter, circuit_breaker, validator, cache, retry_config) = 
            setup_test_environment().await;

        let repo = create_test_repo("cached");
        let batch = vec![repo.clone()];
        
        // Pre-populate cache
        let cache_key = EmbeddingCache::cache_key(&repo.full_name, embedder.model_name());
        cache.put(cache_key, vec![0.1, 0.2, 0.3], embedder.model_name().to_string());
        
        // Process batch - should use cached embedding
        process_batch(
            &batch,
            &client,
            &embedder,
            &rate_limiter,
            &circuit_breaker,
            &validator,
            &cache,
            &retry_config,
        ).await;
        
        // Verify the update was made
        let conn = client.get_connection().await.expect("Failed to get connection");
        let updated: Option<Repo> = conn.select(&repo.id).await.expect("Failed to select repo");
        
        assert!(updated.is_some());
        assert_eq!(updated.unwrap().embedding, Some(vec![0.1, 0.2, 0.3]));
    }

    #[tokio::test]
    async fn test_process_single_repo() {
        let (client, embedder, rate_limiter, circuit_breaker, validator, cache, retry_config) = 
            setup_test_environment().await;

        // Insert test repo into database
        let repo = create_test_repo("single");
        let conn = client.get_connection().await.expect("Failed to get connection");
        let _: Option<Repo> = conn
            .create(("repo", "single"))
            .content(&repo)
            .await
            .expect("Failed to create repo");

        let batch = vec![repo.clone()];
        
        // Mock embedder would be ideal here, but for now we'll just run it
        // In a real test environment, you'd mock the embedder to return predictable results
        
        process_batch(
            &batch,
            &client,
            &embedder,
            &rate_limiter,
            &circuit_breaker,
            &validator,
            &cache,
            &retry_config,
        ).await;
    }

    #[tokio::test]
    async fn test_process_multiple_repos() {
        let (client, embedder, rate_limiter, circuit_breaker, validator, cache, retry_config) = 
            setup_test_environment().await;

        // Insert test repos
        let conn = client.get_connection().await.expect("Failed to get connection");
        let mut batch = Vec::new();
        
        for i in 0..3 {
            let repo = create_test_repo(&format!("multi{}", i));
            let _: Option<Repo> = conn
                .create(("repo", format!("multi{}", i)))
                .content(&repo)
                .await
                .expect("Failed to create repo");
            batch.push(repo);
        }
        
        process_batch(
            &batch,
            &client,
            &embedder,
            &rate_limiter,
            &circuit_breaker,
            &validator,
            &cache,
            &retry_config,
        ).await;
    }

    #[tokio::test]
    async fn test_batch_update_reporting() {
        let (client, embedder, rate_limiter, circuit_breaker, validator, cache, retry_config) = 
            setup_test_environment().await;

        // Create repos with mixed states
        let conn = client.get_connection().await.expect("Failed to get connection");
        
        let repo1 = create_test_repo("update1");
        let _: Option<Repo> = conn.create(("repo", "update1")).content(&repo1).await.expect("Failed to create repo");
        
        let repo2 = create_test_repo("update2");
        let _: Option<Repo> = conn.create(("repo", "update2")).content(&repo2).await.expect("Failed to create repo");
        
        // Pre-cache one to simulate mixed processing
        let cache_key = EmbeddingCache::cache_key(&repo1.full_name, embedder.model_name());
        cache.put(cache_key, vec![0.1, 0.2, 0.3], embedder.model_name().to_string());
        
        let batch = vec![repo1, repo2];
        
        process_batch(
            &batch,
            &client,
            &embedder,
            &rate_limiter,
            &circuit_breaker,
            &validator,
            &cache,
            &retry_config,
        ).await;
    }
}