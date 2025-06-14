use crate::{
    circuit_breaker::CircuitBreakerManager,
    deduplication::{DeduplicationManager, LockGuard},
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
use tracing::{debug, error, info};
use uuid::Uuid;

pub async fn process_batch(
    batch: &[Repo],
    client: &Arc<SurrealClient>,
    embedder: &Arc<Embedder>,
    rate_limiter: &Arc<RateLimiterManager>,
    deduplication: &Arc<DeduplicationManager>,
    circuit_breaker: &Arc<CircuitBreakerManager>,
    validator: &Arc<EmbeddingValidator>,
    cache: &Arc<EmbeddingCache>,
    retry_config: &RetryConfig,
) {
    let batch_id = Uuid::new_v4();
    info!(
        batch_id = %batch_id,
        batch_size = batch.len(),
        "Processing batch"
    );

    // Collect successful updates for batch processing
    let mut pending_updates = Vec::new();
    let mut lock_guards = Vec::new();

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
        let cache_key = EmbeddingCache::cache_key(&repo.full_name, provider);
        
        // Check cache first
        if let Some((cached_embedding, cached_model)) = cache.get(&cache_key) {
            info!("Using cached embedding for {}", repo.full_name);
            
            // Add to pending updates with cached embedding
            pending_updates.push(EmbeddingUpdate {
                repo_id: repo.id.clone(),
                embedding: cached_embedding,
                model: cached_model,
            });
            
            // Store lock guard for later release
            lock_guards.push((repo.id.clone(), lock_guard));
            continue;
        }
        
        // Check rate limit
        if let Err(e) = rate_limiter.wait_for_permit(provider).await {
            error!("Rate limit error: {}", e);
            if let Err(e) = lock_guard.release("failed").await {
                error!("Failed to release lock: {}", e);
            }
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
                            model: embedder.model_name().to_string(),
                        });
                        
                        // Store lock guard for later release
                        lock_guards.push((repo.id.clone(), lock_guard));
                    }
                    Err(e) => {
                        error!("Embedding validation failed for {}: {}", repo.full_name, e);
                        metrics::record_embedding_error(provider, "validation_failed");
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
    
    // Batch update all successful embeddings
    if !pending_updates.is_empty() {
        info!(
            "Batch updating {} embeddings",
            pending_updates.len()
        );
        
        match with_retry(
            "batch_update_embeddings",
            retry_config,
            || async {
                client.batch_update_embeddings(pending_updates.clone()).await
            },
        ).await {
            Ok(result) => {
                info!(
                    "Batch update completed: {} successful, {} failed in {:?}",
                    result.successful,
                    result.failed,
                    result.duration
                );
                
                // Release locks for successful updates
                for (repo_id, guard) in lock_guards {
                    if let Err(e) = guard.release("completed").await {
                        error!("Failed to release lock for {:?}: {}", repo_id, e);
                    }
                }
            }
            Err(e) => {
                error!("Batch update failed: {}", e);
                
                // Release locks with failure status
                for (repo_id, guard) in lock_guards {
                    if let Err(e) = guard.release("failed").await {
                        error!("Failed to release lock for {:?}: {}", repo_id, e);
                    }
                }
            }
        }
    }
}