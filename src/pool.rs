use crate::config::Config;
use anyhow::Result;
use deadpool::{
    managed::{self, Manager, Metrics, Object, RecycleError, RecycleResult},
};
use std::sync::Arc;
use std::time::Duration;
use surrealdb::{
    engine::any::{connect, Any},
    Surreal,
};
use surrealdb::opt::auth::Root;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

/// Type alias for a pooled SurrealDB connection
pub type Connection = Object<SurrealDBManager>;

/// Type alias for the connection pool
pub type Pool = managed::Pool<SurrealDBManager>;

/// Manager for SurrealDB connections that implements deadpool's Manager trait
pub struct SurrealDBManager {
    config: Arc<Config>,
}

impl SurrealDBManager {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    async fn create_connection(&self) -> Result<Surreal<Any>, surrealdb::Error> {
        let url = &self.config.db_url;
        let timeout_duration = Duration::from_secs(self.config.pool_create_timeout_secs);
        
        // Create connection with timeout
        let db: Surreal<Any> = match timeout(timeout_duration, connect(url)).await {
            Ok(Ok(db)) => db,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(surrealdb::Error::Api(
                    surrealdb::error::Api::InternalError(
                        format!("Connection timeout after {:?}", timeout_duration)
                    )
                ));
            }
        };

        // Authenticate with timeout
        match timeout(
            Duration::from_secs(5),
            db.signin(Root {
                username: &self.config.db_user,
                password: &self.config.db_pass,
            })
        ).await {
            Ok(Ok(_)) => {},
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(surrealdb::Error::Api(
                    surrealdb::error::Api::InternalError(
                        "Authentication timeout".to_string()
                    )
                ));
            }
        }

        // Select namespace and database with timeout
        match timeout(
            Duration::from_secs(5),
            db.use_ns(&self.config.db_namespace)
                .use_db(&self.config.db_database)
        ).await {
            Ok(Ok(_)) => {},
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(surrealdb::Error::Api(
                    surrealdb::error::Api::InternalError(
                        "Database selection timeout".to_string()
                    )
                ));
            }
        }

        Ok(db)
    }

    async fn health_check(&self, db: &Surreal<Any>) -> Result<(), surrealdb::Error> {
        // Perform a simple health check query with timeout
        match timeout(Duration::from_secs(5), db.query("RETURN 1")).await {
            Ok(Ok(mut response)) => {
                let _: Option<i32> = response.take(0)?;
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => {
                Err(surrealdb::Error::Api(
                    surrealdb::error::Api::InternalError(
                        "Health check timeout".to_string()
                    )
                ))
            }
        }
    }
}

impl Manager for SurrealDBManager {
    type Type = Surreal<Any>;
    type Error = surrealdb::Error;

    async fn create(&self) -> Result<Self::Type, Self::Error> {
        debug!("Creating new SurrealDB connection");
        let start = std::time::Instant::now();
        
        match self.create_connection().await {
            Ok(conn) => {
                let elapsed = start.elapsed();
                info!("Created new SurrealDB connection in {:?}", elapsed);
                crate::metrics::increment_pool_connections_created();
                Ok(conn)
            }
            Err(e) => {
                error!("Failed to create SurrealDB connection: {}", e);
                crate::metrics::increment_pool_connection_errors();
                Err(e)
            }
        }
    }

    async fn recycle(
        &self,
        conn: &mut Self::Type,
        _: &Metrics,
    ) -> RecycleResult<Self::Error> {
        debug!("Recycling SurrealDB connection");
        
        match self.health_check(conn).await {
            Ok(()) => {
                debug!("Connection health check passed");
                crate::metrics::increment_pool_connections_recycled();
                Ok(())
            }
            Err(e) => {
                warn!("Connection health check failed: {}", e);
                crate::metrics::increment_pool_health_check_failures();
                Err(RecycleError::Message(format!("Health check failed: {}", e).into()))
            }
        }
    }
}

/// Create a connection pool with the given configuration
pub async fn create_pool(config: Arc<Config>) -> Result<Pool> {
    let manager = SurrealDBManager::new(config.clone());
    
    let pool_config = managed::PoolConfig {
        max_size: config.pool_max_size,
        timeouts: managed::Timeouts {
            wait: Some(Duration::from_secs(config.pool_wait_timeout_secs)),
            create: Some(Duration::from_secs(config.pool_create_timeout_secs)),
            recycle: Some(Duration::from_secs(config.pool_recycle_timeout_secs)),
        },
        queue_mode: deadpool::managed::QueueMode::Fifo,
    };

    let pool = managed::Pool::builder(manager)
        .config(pool_config)
        .runtime(deadpool::Runtime::Tokio1)
        .build()?;

    // Pre-warm the pool by creating initial connections
    let initial_size = config.pool_size.min(config.pool_max_size);
    info!("Pre-warming connection pool with {} connections", initial_size);
    
    let mut handles = Vec::new();
    for _ in 0..initial_size {
        let pool_clone = pool.clone();
        let pre_warm_timeout = Duration::from_secs(config.pool_create_timeout_secs);
        handles.push(tokio::spawn(async move {
            match timeout(pre_warm_timeout, pool_clone.get()).await {
                Ok(Ok(_)) => {
                    debug!("Successfully pre-warmed connection");
                }
                Ok(Err(e)) => {
                    warn!("Failed to pre-warm connection: {}", e);
                }
                Err(_) => {
                    warn!("Pre-warm connection timeout after {:?}", pre_warm_timeout);
                }
            }
        }));
    }
    
    // Wait for all pre-warming tasks to complete with timeout
    let pre_warm_total_timeout = Duration::from_secs(config.pool_create_timeout_secs * 2);
    if let Err(_) = timeout(pre_warm_total_timeout, async {
        for handle in handles {
            let _ = handle.await;
        }
    }).await {
        warn!("Pre-warming phase timed out, continuing anyway");
    }
    
    info!(
        "Connection pool created with max size: {}, initial size: {}",
        config.pool_max_size, initial_size
    );
    
    Ok(pool)
}

/// Extension trait for Pool to provide convenience methods
pub trait PoolExt {
    /// Get pool statistics
    fn stats(&self) -> PoolStats;
}

/// Pool statistics
#[derive(Debug, Clone)]
pub struct PoolStats {
    pub size: usize,
    pub available: usize,
    pub waiting: usize,
    pub max_size: usize,
}

impl PoolExt for Pool {
    fn stats(&self) -> PoolStats {
        let status = self.status();
        PoolStats {
            size: status.size as usize,
            available: status.available as usize,
            waiting: status.waiting as usize,
            max_size: status.max_size as usize,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::sync::Arc;

    fn test_config() -> Arc<Config> {
        Arc::new(Config {
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
        })
    }

    #[tokio::test]
    async fn test_surreal_manager_create_connection() {
        let config = test_config();
        let manager = SurrealDBManager::new(config);

        // Test connection creation
        let result = manager.create_connection().await;
        assert!(result.is_ok(), "Failed to create connection: {:?}", result.err());

        let conn = result.unwrap();
        // Verify we can perform a simple query
        let query_result = conn.query("RETURN 1").await;
        assert!(query_result.is_ok());
    }

    #[tokio::test]
    async fn test_surreal_manager_health_check() {
        let config = test_config();
        let manager = SurrealDBManager::new(config);

        // Create a connection
        let conn = manager.create_connection().await.expect("Failed to create connection");
        
        // Test health check
        let health_result = manager.health_check(&conn).await;
        assert!(health_result.is_ok(), "Health check failed: {:?}", health_result.err());
    }

    #[tokio::test]
    async fn test_pool_creation() {
        let config = test_config();
        let pool_result = create_pool(config.clone()).await;
        
        assert!(pool_result.is_ok(), "Failed to create pool: {:?}", pool_result.err());
        
        let pool = pool_result.unwrap();
        let stats = pool.stats();
        
        // Verify pool was created with correct configuration
        assert_eq!(stats.max_size, config.pool_max_size);
        assert!(stats.size >= config.pool_size.min(config.pool_max_size));
    }

    #[tokio::test]
    async fn test_pool_get_connection() {
        let config = test_config();
        let pool = create_pool(config).await.expect("Failed to create pool");
        
        // Get a connection from the pool
        let conn_result = pool.get().await;
        assert!(conn_result.is_ok(), "Failed to get connection: {:?}", conn_result.err());
        
        let conn = conn_result.unwrap();
        
        // Verify the connection works
        let mut response = conn.query("SELECT 1 as value").await.expect("Query failed");
        let result: Option<serde_json::Value> = response.take(0).expect("Failed to get result");
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn test_pool_concurrent_access() {
        let config = Arc::new(Config {
            pool_max_size: 3,
            pool_size: 2,
            ..test_config().as_ref().clone()
        });
        
        let pool = create_pool(config).await.expect("Failed to create pool");
        
        // Spawn multiple tasks to access the pool concurrently
        let mut handles = Vec::new();
        for i in 0..5 {
            let pool_clone = pool.clone();
            let handle = tokio::spawn(async move {
                let conn = pool_clone.get().await.expect("Failed to get connection");
                
                // Hold the connection for a bit
                tokio::time::sleep(Duration::from_millis(10)).await;
                
                // Perform a query
                let mut response = conn.query("SELECT $id as id")
                    .bind(("id", i))
                    .await
                    .expect("Query failed");
                let result: Option<serde_json::Value> = response.take(0).expect("Failed to get result");
                assert!(result.is_some());
            });
            handles.push(handle);
        }
        
        // Wait for all tasks to complete
        for handle in handles {
            handle.await.expect("Task failed");
        }
    }

    #[tokio::test]
    async fn test_pool_stats() {
        let config = Arc::new(Config {
            pool_max_size: 5,
            pool_size: 2,
            ..test_config().as_ref().clone()
        });
        
        let pool = create_pool(config.clone()).await.expect("Failed to create pool");
        
        let initial_stats = pool.stats();
        assert_eq!(initial_stats.max_size, 5);
        assert!(initial_stats.size >= 2);
        
        // Get a connection and check stats again
        let _conn = pool.get().await.expect("Failed to get connection");
        let stats_with_conn = pool.stats();
        assert!(stats_with_conn.available < initial_stats.available);
    }

    #[tokio::test]
    async fn test_connection_recycling() {
        let config = test_config();
        let manager = SurrealDBManager::new(config);
        
        // Create and test recycling
        let mut conn = manager.create().await.expect("Failed to create connection");
        let metrics = deadpool::managed::Metrics::default();
        
        // First recycle should succeed
        let recycle_result = manager.recycle(&mut conn, &metrics).await;
        assert!(recycle_result.is_ok(), "Recycle failed: {:?}", recycle_result.err());
        
        // Connection should still be usable after recycling
        let query_result = conn.query("RETURN 1").await;
        assert!(query_result.is_ok());
    }

    #[tokio::test]
    async fn test_pool_timeout() {
        let config = Arc::new(Config {
            pool_max_size: 1,
            pool_size: 1,
            pool_wait_timeout_secs: 1, // Short timeout
            ..test_config().as_ref().clone()
        });
        
        let pool = create_pool(config).await.expect("Failed to create pool");
        
        // Get the only available connection
        let _conn1 = pool.get().await.expect("Failed to get first connection");
        
        // Try to get another connection (should timeout)
        let start = std::time::Instant::now();
        let conn2_result = pool.get().await;
        let elapsed = start.elapsed();
        
        assert!(conn2_result.is_err());
        assert!(elapsed >= Duration::from_secs(1));
        assert!(elapsed < Duration::from_secs(2)); // Should timeout quickly
    }
}