use crate::{metrics, pool::Pool};
use tokio::time::{interval, Duration};
use tracing::error;

/// Monitor connection pool statistics
pub async fn monitor_pool_metrics(
    pool: Pool,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) {
    let mut interval = interval(Duration::from_secs(30));

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                break;
            }
            _ = interval.tick() => {
                report_pool_metrics(&pool).await;
            }
        }
    }
}

async fn report_pool_metrics(pool: &Pool) {
    // For SurrealDB, we track basic connection health
    match pool.health().await {
        Ok(_) => {
            metrics::update_active_connections("surrealdb", 1);
        }
        Err(e) => {
            error!("Database connection unhealthy: {}", e);
            metrics::update_active_connections("surrealdb", 0);
        }
    }
}

/// Connection pool statistics
#[derive(Debug, Clone)]
pub struct PoolStats {
    pub total_connections: u32,
    pub active_connections: u32,
    pub idle_connections: u32,
    pub wait_count: u64,
    pub wait_duration: Duration,
}

impl Default for PoolStats {
    fn default() -> Self {
        Self {
            total_connections: 1,
            active_connections: 0,
            idle_connections: 1,
            wait_count: 0,
            wait_duration: Duration::from_secs(0),
        }
    }
}