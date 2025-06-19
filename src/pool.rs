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
        
        // Create connection using the Any engine
        // This automatically detects the connection type from the URL scheme:
        // - ws:// or wss:// for WebSocket connections
        // - http:// or https:// for HTTP connections
        // - memory:// for in-memory databases
        // - rocksdb:// for embedded RocksDB databases
        // - tikv:// for TiKV connections
        // - fdb:// for FoundationDB connections
        let db: Surreal<Any> = connect(url).await?;

        // Authenticate
        db.signin(Root {
            username: &self.config.db_user,
            password: &self.config.db_pass,
        })
        .await?;

        // Select namespace and database
        db.use_ns(&self.config.db_namespace)
            .use_db(&self.config.db_database)
            .await?;

        Ok(db)
    }

    async fn health_check(&self, db: &Surreal<Any>) -> Result<(), surrealdb::Error> {
        // Perform a simple health check query
        let mut response = db
            .query("RETURN 1")
            .await?;
        let _: Option<i32> = response.take(0)?;
        
        Ok(())
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
        handles.push(tokio::spawn(async move {
            match pool_clone.get().await {
                Ok(_) => {
                    debug!("Successfully pre-warmed connection");
                }
                Err(e) => {
                    warn!("Failed to pre-warm connection: {}", e);
                }
            }
        }));
    }
    
    // Wait for all pre-warming tasks to complete
    for handle in handles {
        let _ = handle.await;
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