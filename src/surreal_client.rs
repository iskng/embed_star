use crate::{
    models::Repo,
    pool::Pool,
    error::{EmbedError, Result},
};
use surrealdb::sql::Thing;
use tracing::{debug, error, info, warn};
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
        repo_id: &Thing,
        embedding: Vec<f32>,
        model: &str,
    ) -> Result<()> {
        let db = self.pool.clone();

        let query = r#"
            UPDATE $repo_id SET
                embedding = $embedding,
                embedding_model = $model,
                embedding_generated_at = time::now()
        "#;

        let result: Option<Repo> = db
            .query(query)
            .bind(("repo_id", repo_id))
            .bind(("embedding", embedding))
            .bind(("model", model))
            .await?
            .take(0)?;

        match result {
            Some(repo) => {
                debug!(
                    "Updated embedding for repo {}: {} dimensions",
                    repo.full_name,
                    repo.embedding.as_ref().map(|e| e.len()).unwrap_or(0)
                );
                Ok(())
            }
            None => {
                warn!("Failed to update embedding for repo {:?}", repo_id);
                Err(EmbedError::Database(surrealdb::Error::Db(surrealdb::error::Db::RecordExists {
                    thing: repo_id.to_string(),
                })))
            }
        }
    }

    pub async fn get_repos_needing_embeddings(&self, limit: usize) -> Result<Vec<Repo>> {
        let db = self.pool.clone();

        let query = r#"
            SELECT * FROM repo
            WHERE embedding IS NULL
                OR (updated_at > embedding_generated_at)
            LIMIT $limit
        "#;

        let repos: Vec<Repo> = db
            .query(query)
            .bind(("limit", limit))
            .await?
            .take(0)?;

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
            
            loop {
                interval.tick().await;
                
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
        let db = self.pool.clone();
        let result: Option<usize> = db
            .query("SELECT count() FROM repo GROUP ALL")
            .await?
            .take(0)?;
        Ok(result.unwrap_or(0))
    }

    pub async fn get_embedded_repos_count(&self) -> Result<usize> {
        let db = self.pool.clone();
        let result: Option<usize> = db
            .query("SELECT count() FROM repo WHERE embedding IS NOT NULL GROUP ALL")
            .await?
            .take(0)?;
        Ok(result.unwrap_or(0))
    }

    pub async fn get_pending_repos_count(&self) -> Result<usize> {
        let db = self.pool.clone();
        let query = r#"
            SELECT count() FROM repo
            WHERE embedding IS NULL
                OR (updated_at > embedding_generated_at)
            GROUP ALL
        "#;
        let result: Option<usize> = db.query(query).await?.take(0)?;
        Ok(result.unwrap_or(0))
    }

    /// Batch update multiple repository embeddings in a single transaction
    pub async fn batch_update_embeddings(
        &self,
        updates: Vec<EmbeddingUpdate>,
    ) -> Result<BatchUpdateResult> {
        if updates.is_empty() {
            return Ok(BatchUpdateResult::default());
        }

        let start = Instant::now();
        let db = self.pool.clone();
        let batch_size = updates.len();

        // Build a transaction with all updates
        let mut query = String::from("BEGIN TRANSACTION;");
        
        for (idx, update) in updates.iter().enumerate() {
            query.push_str(&format!(
                r#"
                UPDATE {} SET
                    embedding = $embedding_{},
                    embedding_model = $model_{},
                    embedding_generated_at = time::now();
                "#,
                update.repo_id, idx, idx
            ));
        }
        
        query.push_str("COMMIT TRANSACTION;");

        // Bind all parameters
        let mut statement = db.query(&query);
        for (idx, update) in updates.iter().enumerate() {
            statement = statement
                .bind((format!("embedding_{}", idx), update.embedding.clone()))
                .bind((format!("model_{}", idx), update.model.clone()));
        }

        // Execute the batch update
        match statement.await {
            Ok(_) => {
                let duration = start.elapsed();
                info!(
                    "Batch updated {} embeddings in {:?} ({:.2} updates/sec)",
                    batch_size,
                    duration,
                    batch_size as f64 / duration.as_secs_f64()
                );
                
                Ok(BatchUpdateResult {
                    total: batch_size,
                    successful: batch_size,
                    failed: 0,
                    duration,
                })
            }
            Err(e) => {
                error!("Batch update failed: {}", e);
                // Fall back to individual updates
                self.fallback_individual_updates(updates).await
            }
        }
    }

    /// Fallback to individual updates if batch update fails
    async fn fallback_individual_updates(
        &self,
        updates: Vec<EmbeddingUpdate>,
    ) -> Result<BatchUpdateResult> {
        let start = Instant::now();
        let mut successful = 0;
        let mut failed = 0;

        for update in updates {
            match self.update_repo_embedding(&update.repo_id, update.embedding, &update.model).await {
                Ok(_) => successful += 1,
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
}

/// Represents a single embedding update
#[derive(Debug, Clone)]
pub struct EmbeddingUpdate {
    pub repo_id: Thing,
    pub embedding: Vec<f32>,
    pub model: String,
}

/// Result of a batch update operation
#[derive(Debug, Default)]
pub struct BatchUpdateResult {
    pub total: usize,
    pub successful: usize,
    pub failed: usize,
    pub duration: std::time::Duration,
}