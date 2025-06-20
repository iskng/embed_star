use crate::{ models::Repo, pool::{ Pool, PoolExt }, error::{ EmbedError, Result } };
use serde_json;
use surrealdb::RecordId;
use tracing::{ debug, error, info, warn };
use std::time::Instant;

#[derive(Clone)]
pub struct SurrealClient {
    pool: Pool,
}

impl SurrealClient {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    pub async fn update_repo_embedding(
        &self,
        repo_id: &RecordId,
        embedding: Vec<f32>
    ) -> Result<()> {
        // Get a connection from the pool
        let conn = self.pool
            .get().await
            .map_err(|e|
                EmbedError::Database(
                    surrealdb::Error::Api(surrealdb::error::Api::InternalError(e.to_string()))
                )
            )?;

        let query =
            r#"
            UPDATE $repo_id SET
                embedding = $embedding,
                embedding_generated_at = time::now()
        "#;

        let mut response = conn
            .query(query)
            .bind(("repo_id", repo_id.clone()))
            .bind(("embedding", embedding)).await?;
        let result: Option<Repo> = response.take(0)?;

        match result {
            Some(repo) => {
                debug!(
                    "Updated embedding for repo {}: {} dimensions",
                    repo.full_name,
                    repo.embedding
                        .as_ref()
                        .map(|e| e.len())
                        .unwrap_or(0)
                );
                Ok(())
            }
            None => {
                warn!("Failed to update embedding for repo {:?}", repo_id);
                Err(
                    EmbedError::Database(
                        surrealdb::Error::Api(
                            surrealdb::error::Api::InternalError(
                                format!("Record not found and could not be updated: {}", repo_id)
                            )
                        )
                    )
                )
            }
        }
    }

    pub async fn get_repos_needing_embeddings(&self, limit: usize) -> Result<Vec<Repo>> {
        // Get a connection from the pool
        let conn = self.pool
            .get().await
            .map_err(|e|
                EmbedError::Database(
                    surrealdb::Error::Api(surrealdb::error::Api::InternalError(e.to_string()))
                )
            )?;

        let query =
            r#"
            SELECT * FROM repo
            WHERE embedding IS NONE
                OR (updated_at > embedding_generated_at)
            LIMIT $limit
        "#;

        let mut response = conn.query(query).bind(("limit", limit)).await?;
        let repos: Vec<Repo> = response.take(0)?;

        Ok(repos)
    }

    pub async fn setup_live_query(&self) -> Result<tokio::sync::mpsc::Receiver<Repo>> {
        // For now, we'll use a polling approach instead of live queries
        // as the API for live queries has changed significantly
        let (tx, rx) = tokio::sync::mpsc::channel(100);

        info!("Starting polling for repos needing embeddings");

        let client = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
            let mut processed_ids = std::collections::HashSet::new();
            let mut clear_counter = 0;
            const MAX_PROCESSED_IDS: usize = 10000;
            const CLEAR_INTERVAL: u32 = 100; // Clear every 100 iterations (500 seconds)

            loop {
                interval.tick().await;
                clear_counter += 1;

                // Periodically clear the processed IDs to prevent unbounded growth
                if clear_counter >= CLEAR_INTERVAL || processed_ids.len() > MAX_PROCESSED_IDS {
                    debug!("Clearing processed IDs cache (size was: {})", processed_ids.len());
                    processed_ids.clear();
                    clear_counter = 0;
                }

                match client.get_repos_needing_embeddings(50).await {
                    Ok(repos) => {
                        for repo in repos {
                            if !processed_ids.contains(&repo.id) {
                                processed_ids.insert(repo.id.clone());
                                if tx.send(repo).await.is_err() {
                                    error!("Failed to send repo through channel");
                                    return;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Error fetching repos needing embeddings: {}", e);
                    }
                }
            }
        });

        Ok(rx)
    }

    pub async fn get_total_repos_count(&self) -> Result<usize> {
        // Get a connection from the pool
        let conn = self.pool
            .get().await
            .map_err(|e|
                EmbedError::Database(
                    surrealdb::Error::Api(surrealdb::error::Api::InternalError(e.to_string()))
                )
            )?;

        let mut response = conn.query("SELECT count() FROM repo GROUP ALL").await?;
        // SurrealDB 2.3 returns count as { "count": value }
        let result: Option<serde_json::Value> = response.take(0)?;
        match result {
            Some(val) => {
                if let Some(count) = val.get("count").and_then(|v| v.as_i64()) {
                    Ok(count as usize)
                } else {
                    Ok(0)
                }
            }
            None => Ok(0)
        }
    }

    pub async fn get_embedded_repos_count(&self) -> Result<usize> {
        // Get a connection from the pool
        let conn = self.pool
            .get().await
            .map_err(|e|
                EmbedError::Database(
                    surrealdb::Error::Api(surrealdb::error::Api::InternalError(e.to_string()))
                )
            )?;

        let mut response = conn.query(
            "SELECT count() FROM repo WHERE embedding IS NOT NONE GROUP ALL"
        ).await?;
        // SurrealDB 2.3 returns count as { "count": value }
        let result: Option<serde_json::Value> = response.take(0)?;
        match result {
            Some(val) => {
                if let Some(count) = val.get("count").and_then(|v| v.as_i64()) {
                    Ok(count as usize)
                } else {
                    Ok(0)
                }
            }
            None => Ok(0)
        }
    }

    pub async fn get_pending_repos_count(&self) -> Result<usize> {
        // Get a connection from the pool
        let conn = self.pool
            .get().await
            .map_err(|e|
                EmbedError::Database(
                    surrealdb::Error::Api(surrealdb::error::Api::InternalError(e.to_string()))
                )
            )?;

        let query =
            r#"
            SELECT count() FROM repo
            WHERE embedding IS NONE
                OR (updated_at > embedding_generated_at)
            GROUP ALL
        "#;
        let mut response = conn.query(query).await?;
        // SurrealDB 2.3 returns count as { "count": value }
        let result: Option<serde_json::Value> = response.take(0)?;
        match result {
            Some(val) => {
                if let Some(count) = val.get("count").and_then(|v| v.as_i64()) {
                    Ok(count as usize)
                } else {
                    Ok(0)
                }
            }
            None => Ok(0)
        }
    }

    /// Batch update multiple repository embeddings in a single transaction
    pub async fn batch_update_embeddings(
        &self,
        updates: Vec<EmbeddingUpdate>
    ) -> Result<BatchUpdateResult> {
        if updates.is_empty() {
            return Ok(BatchUpdateResult::default());
        }

        let start = Instant::now();
        let total = updates.len();

        // Try to use proper batch update with transaction
        match self.batch_update_with_transaction(updates.clone()).await {
            Ok(successful) => {
                Ok(BatchUpdateResult {
                    total,
                    successful,
                    failed: total - successful,
                    duration: start.elapsed(),
                })
            }
            Err(e) => {
                warn!("Batch update failed, falling back to individual updates: {}", e);
                // Fallback to individual updates if batch fails
                self.fallback_individual_updates(updates).await
            }
        }
    }

    /// Perform batch updates using a transaction
    async fn batch_update_with_transaction(
        &self,
        updates: Vec<EmbeddingUpdate>
    ) -> Result<usize> {
        let conn = self.pool.get().await
            .map_err(|e| EmbedError::Database(
                surrealdb::Error::Api(surrealdb::error::Api::InternalError(e.to_string()))
            ))?;

        // Build a single query with all updates
        let mut query = String::from("BEGIN TRANSACTION;\n");
        
        for (idx, _) in updates.iter().enumerate() {
            query.push_str(&format!(
                "UPDATE $repo_{} SET embedding = $embedding_{}, embedding_generated_at = time::now();\n",
                idx, idx
            ));
        }
        
        query.push_str("COMMIT TRANSACTION;");

        // Create query and bind parameters
        let mut bound_query = conn.query(query);
        for (idx, update) in updates.iter().enumerate() {
            bound_query = bound_query
                .bind((format!("repo_{}", idx), update.repo_id.clone()))
                .bind((format!("embedding_{}", idx), update.embedding.clone()));
        }

        // Execute the transaction
        let _response = bound_query.await?;
        
        // Count successful updates
        let successful = updates.len(); // If transaction succeeds, all updates succeeded
        
        Ok(successful)
    }

    /// Fallback to individual updates if batch update fails
    async fn fallback_individual_updates(
        &self,
        updates: Vec<EmbeddingUpdate>
    ) -> Result<BatchUpdateResult> {
        let start = Instant::now();
        let mut successful = 0;
        let mut failed = 0;

        for update in updates {
            match
                self.update_repo_embedding(&update.repo_id, update.embedding).await
            {
                Ok(_) => {
                    successful += 1;
                }
                Err(e) => {
                    error!("Failed to update embedding for {:?}: {}", update.repo_id, e);
                    failed += 1;
                }
            }
        }

        Ok(BatchUpdateResult {
            total: successful + failed,
            successful,
            failed,
            duration: start.elapsed(),
        })
    }

    /// Get current pool statistics
    pub fn get_pool_stats(&self) -> crate::pool::PoolStats {
        self.pool.stats()
    }
}

/// Represents a single embedding update
#[derive(Debug, Clone)]
pub struct EmbeddingUpdate {
    pub repo_id: RecordId,
    pub embedding: Vec<f32>,
}

/// Result of a batch update operation
#[derive(Debug, Default)]
pub struct BatchUpdateResult {
    pub total: usize,
    pub successful: usize,
    pub failed: usize,
    pub duration: std::time::Duration,
}
