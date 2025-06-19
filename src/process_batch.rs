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