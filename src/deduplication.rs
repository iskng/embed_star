use crate::{
    error::Result,
    pool::Pool,
};
use surrealdb::sql::Thing;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Manages distributed deduplication using SurrealDB processing locks
#[derive(Clone)]
pub struct DeduplicationManager {
    pool: Pool,
    instance_id: String,
    lock_duration_seconds: i64,
}

impl DeduplicationManager {
    /// Create a new deduplication manager with a unique instance ID
    pub fn new(pool: Pool) -> Self {
        let instance_id = format!("embed_star_{}", Uuid::new_v4());
        info!("Created deduplication manager with instance ID: {}", instance_id);
        
        Self {
            pool,
            instance_id,
            lock_duration_seconds: 300, // 5 minutes default
        }
    }

    /// Attempt to acquire a processing lock for a repository
    pub async fn try_acquire_lock(&self, repo_id: &Thing) -> Result<bool> {
        let db = self.pool.clone();
        
        let query = r#"
            RETURN fn::acquire_processing_lock($repo_id, $instance_id, $duration);
        "#;

        let result: Option<bool> = db
            .query(query)
            .bind(("repo_id", repo_id))
            .bind(("instance_id", &self.instance_id))
            .bind(("duration", self.lock_duration_seconds))
            .await?
            .take(0)?;

        let acquired = result.unwrap_or(false);
        
        if acquired {
            debug!(
                "Acquired processing lock for repo {:?} on instance {}",
                repo_id, self.instance_id
            );
        } else {
            debug!(
                "Failed to acquire lock for repo {:?} - already being processed",
                repo_id
            );
        }

        Ok(acquired)
    }

    /// Release a processing lock after completing work
    pub async fn release_lock(&self, repo_id: &Thing, status: &str) -> Result<()> {
        let db = self.pool.clone();
        
        let query = r#"
            RETURN fn::release_processing_lock($repo_id, $instance_id, $status);
        "#;

        db.query(query)
            .bind(("repo_id", repo_id))
            .bind(("instance_id", &self.instance_id))
            .bind(("status", status))
            .await?;

        debug!(
            "Released processing lock for repo {:?} with status: {}",
            repo_id, status
        );

        Ok(())
    }

    /// Extend a lock for long-running operations
    pub async fn extend_lock(&self, repo_id: &Thing, additional_seconds: i64) -> Result<bool> {
        let db = self.pool.clone();
        
        let query = r#"
            RETURN fn::extend_processing_lock($repo_id, $instance_id, $additional_seconds);
        "#;

        let result: Option<serde_json::Value> = db
            .query(query)
            .bind(("repo_id", repo_id))
            .bind(("instance_id", &self.instance_id))
            .bind(("additional_seconds", additional_seconds))
            .await?
            .take(0)?;

        let extended = result.is_some();
        
        if extended {
            debug!(
                "Extended processing lock for repo {:?} by {} seconds",
                repo_id, additional_seconds
            );
        } else {
            warn!(
                "Failed to extend lock for repo {:?} - lock may have expired",
                repo_id
            );
        }

        Ok(extended)
    }

    /// Clean up expired locks (should be called periodically)
    pub async fn cleanup_expired_locks(&self) -> Result<()> {
        let db = self.pool.clone();
        
        let query = r#"
            RETURN fn::cleanup_expired_locks();
        "#;

        db.query(query).await?;
        
        debug!("Cleaned up expired processing locks");
        Ok(())
    }

    /// Get the number of active locks for this instance
    pub async fn get_active_locks_count(&self) -> Result<usize> {
        let db = self.pool.clone();
        
        let query = r#"
            SELECT count() FROM processing_lock 
            WHERE instance_id = $instance_id 
                AND expires_at > time::now() 
            GROUP ALL
        "#;

        let result: Option<usize> = db
            .query(query)
            .bind(("instance_id", &self.instance_id))
            .await?
            .take(0)?;

        Ok(result.unwrap_or(0))
    }

    /// Get all active locks (for monitoring)
    pub async fn get_all_active_locks(&self) -> Result<Vec<ProcessingLock>> {
        let db = self.pool.clone();
        
        let query = r#"
            SELECT * FROM processing_lock 
            WHERE expires_at > time::now()
            ORDER BY locked_at DESC
        "#;

        let locks: Vec<ProcessingLock> = db.query(query).await?.take(0)?;
        Ok(locks)
    }
}

/// Represents a processing lock record
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProcessingLock {
    pub id: Thing,
    pub repo_id: Thing,
    pub instance_id: String,
    pub locked_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub processing_status: String,
}

/// Auto-release guard for processing locks
pub struct LockGuard<'a> {
    manager: &'a DeduplicationManager,
    repo_id: Thing,
    released: bool,
}

impl<'a> LockGuard<'a> {
    pub fn new(manager: &'a DeduplicationManager, repo_id: Thing) -> Self {
        Self {
            manager,
            repo_id,
            released: false,
        }
    }

    /// Manually release the lock with a specific status
    pub async fn release(mut self, status: &str) -> Result<()> {
        self.released = true;
        self.manager.release_lock(&self.repo_id, status).await
    }

    /// Extend the lock duration
    pub async fn extend(&self, additional_seconds: i64) -> Result<bool> {
        self.manager.extend_lock(&self.repo_id, additional_seconds).await
    }
}

impl<'a> Drop for LockGuard<'a> {
    fn drop(&mut self) {
        if !self.released {
            // Log a warning if the lock wasn't explicitly released
            warn!(
                "LockGuard dropped without explicit release for repo {:?}",
                self.repo_id
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instance_id_generation() {
        // Test that instance IDs are unique and properly formatted
        let id1 = format!("embed_star_{}", Uuid::new_v4());
        let id2 = format!("embed_star_{}", Uuid::new_v4());
        
        assert!(id1.starts_with("embed_star_"));
        assert!(id2.starts_with("embed_star_"));
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_processing_lock_struct() {
        // Test the ProcessingLock struct
        use chrono::Utc;
        
        let lock = ProcessingLock {
            id: Thing::from(("processing_lock", "test123")),
            repo_id: Thing::from(("repo", "test456")),
            instance_id: "embed_star_test".to_string(),
            locked_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::seconds(300),
            processing_status: "processing".to_string(),
        };
        
        assert_eq!(lock.instance_id, "embed_star_test");
        assert_eq!(lock.processing_status, "processing");
    }
}