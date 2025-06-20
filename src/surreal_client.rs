use crate::{ models::Repo, pool::{ Pool, PoolExt }, error::{ EmbedError, Result } };
use serde_json;
use surrealdb::RecordId;
use tracing::{ debug, error, info, warn };
use std::time::Instant;
#[cfg(test)]
use deadpool::managed::Object;

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
    
    #[cfg(test)]
    pub async fn get_connection(&self) -> Result<Object<crate::pool::SurrealDBManager>> {
        self.pool.get().await.map_err(|e| EmbedError::Database(
            surrealdb::Error::Api(surrealdb::error::Api::InternalError(e.to_string()))
        ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, models::Repo};
    use chrono::Utc;
    use std::sync::Arc;

    async fn setup_test_client() -> (SurrealClient, Pool) {
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
            retry_attempts: 3,
            retry_delay_ms: 100,
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

        let pool = crate::pool::create_pool(config).await.expect("Failed to create pool");
        
        // Create test table
        let conn = pool.get().await.expect("Failed to get connection");
        conn.query("DEFINE TABLE repo SCHEMALESS").await.expect("Failed to create table");
        
        let client = SurrealClient::new(pool.clone());
        (client, pool)
    }

    fn create_test_repo(id: &str, needs_embedding: bool) -> Repo {
        let now = Utc::now();
        Repo {
            id: RecordId::from(("repo", id)),
            github_id: 123456,
            name: format!("test-{}", id),
            full_name: format!("owner/test-{}", id),
            description: Some("Test repository".to_string()),
            url: format!("https://github.com/owner/test-{}", id),
            stars: 42,
            language: Some("Rust".to_string()),
            owner: crate::models::RepoOwner {
                login: "owner".to_string(),
                avatar_url: "https://github.com/owner.png".to_string(),
            },
            is_private: false,
            created_at: now,
            updated_at: now,
            embedding: if needs_embedding { None } else { Some(vec![0.1, 0.2, 0.3]) },
            embedding_generated_at: if needs_embedding { None } else { Some(now) },
        }
    }

    #[tokio::test]
    async fn test_update_repo_embedding() {
        let (client, _pool) = setup_test_client().await;
        
        // Insert a test repo
        let repo = create_test_repo("test1", true);
        let conn = client.get_connection().await.expect("Failed to get connection");
        let _: Option<Repo> = conn
            .create(("repo", "test1"))
            .content(&repo)
            .await
            .expect("Failed to create repo");
        
        // Update embedding
        let embedding = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let result = client.update_repo_embedding(&repo.id, embedding.clone()).await;
        
        assert!(result.is_ok(), "Failed to update embedding: {:?}", result.err());
        
        // Verify the update
        let updated: Option<Repo> = conn
            .select(&repo.id)
            .await
            .expect("Failed to select repo");
        
        assert!(updated.is_some());
        let updated_repo = updated.unwrap();
        assert_eq!(updated_repo.embedding, Some(embedding));
        assert!(updated_repo.embedding_generated_at.is_some());
    }

    #[tokio::test]
    async fn test_get_repos_needing_embeddings() {
        let (client, pool) = setup_test_client().await;
        let conn = pool.get().await.expect("Failed to get connection");
        
        // Insert test repos
        let repo1 = create_test_repo("needs1", true);
        let repo2 = create_test_repo("needs2", true);
        let repo3 = create_test_repo("has_embedding", false);
        
        let _: Option<Repo> = conn.create(("repo", "needs1")).content(&repo1).await.expect("Failed to create repo");
        let _: Option<Repo> = conn.create(("repo", "needs2")).content(&repo2).await.expect("Failed to create repo");
        let _: Option<Repo> = conn.create(("repo", "has_embedding")).content(&repo3).await.expect("Failed to create repo");
        
        // Get repos needing embeddings
        let repos = client.get_repos_needing_embeddings(10).await.expect("Failed to get repos");
        
        assert_eq!(repos.len(), 2);
        assert!(repos.iter().any(|r| r.full_name == "owner/test-needs1"));
        assert!(repos.iter().any(|r| r.full_name == "owner/test-needs2"));
        assert!(!repos.iter().any(|r| r.full_name == "owner/test-has_embedding"));
    }

    #[tokio::test]
    async fn test_batch_update_embeddings() {
        let (client, pool) = setup_test_client().await;
        let conn = pool.get().await.expect("Failed to get connection");
        
        // Insert test repos
        let repo1 = create_test_repo("batch1", true);
        let repo2 = create_test_repo("batch2", true);
        
        let _: Option<Repo> = conn.create(("repo", "batch1")).content(&repo1).await.expect("Failed to create repo");
        let _: Option<Repo> = conn.create(("repo", "batch2")).content(&repo2).await.expect("Failed to create repo");
        
        // Prepare batch updates
        let updates = vec![
            EmbeddingUpdate {
                repo_id: repo1.id.clone(),
                embedding: vec![0.1, 0.2, 0.3],
            },
            EmbeddingUpdate {
                repo_id: repo2.id.clone(),
                embedding: vec![0.4, 0.5, 0.6],
            },
        ];
        
        // Perform batch update
        let result = client.batch_update_embeddings(updates).await.expect("Batch update failed");
        
        assert_eq!(result.total, 2);
        assert_eq!(result.successful, 2);
        assert_eq!(result.failed, 0);
        
        // Verify updates
        let updated1: Option<Repo> = conn.select(&repo1.id).await.expect("Failed to select repo");
        let updated2: Option<Repo> = conn.select(&repo2.id).await.expect("Failed to select repo");
        
        assert!(updated1.unwrap().embedding.is_some());
        assert!(updated2.unwrap().embedding.is_some());
    }

    #[tokio::test]
    async fn test_get_counts() {
        let (client, pool) = setup_test_client().await;
        let conn = pool.get().await.expect("Failed to get connection");
        
        // Insert test repos
        let repo1 = create_test_repo("count1", true);
        let repo2 = create_test_repo("count2", false);
        let repo3 = create_test_repo("count3", true);
        
        let _: Option<Repo> = conn.create(("repo", "count1")).content(&repo1).await.expect("Failed to create repo");
        let _: Option<Repo> = conn.create(("repo", "count2")).content(&repo2).await.expect("Failed to create repo");
        let _: Option<Repo> = conn.create(("repo", "count3")).content(&repo3).await.expect("Failed to create repo");
        
        // Test counts
        let total = client.get_total_repos_count().await.expect("Failed to get total count");
        let embedded = client.get_embedded_repos_count().await.expect("Failed to get embedded count");
        let pending = client.get_pending_repos_count().await.expect("Failed to get pending count");
        
        assert_eq!(total, 3);
        assert_eq!(embedded, 1);
        assert_eq!(pending, 2);
    }

    #[tokio::test]
    async fn test_empty_batch_update() {
        let (client, _pool) = setup_test_client().await;
        
        // Test empty batch
        let result = client.batch_update_embeddings(vec![]).await.expect("Empty batch update failed");
        
        assert_eq!(result.total, 0);
        assert_eq!(result.successful, 0);
        assert_eq!(result.failed, 0);
    }

    #[tokio::test]
    async fn test_pool_stats() {
        let (client, _pool) = setup_test_client().await;
        
        let stats = client.get_pool_stats();
        assert!(stats.max_size > 0);
        assert!(stats.size <= stats.max_size);
    }
}
