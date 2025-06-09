use crate::{
    models::Repo,
    pool::Pool,
};
use anyhow::Result;
use surrealdb::sql::Thing;
use tracing::{debug, error, info, warn};

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
                Err(anyhow::anyhow!("Failed to update repo"))
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
}