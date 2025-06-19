use crate::{metrics, pool::{Pool, PoolExt}};
use tokio::time::{interval, Duration};
use tracing::{debug, error};

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
    // Get pool statistics
    let stats = pool.stats();
    
    debug!(
        "Pool stats - size: {}, available: {}, waiting: {}, max: {}",
        stats.size, stats.available, stats.waiting, stats.max_size
    );
    
    // Update metrics
    metrics::update_active_connections("surrealdb", stats.size as i64);
    metrics::set_pool_connections_active(stats.size as i64 - stats.available as i64);
    metrics::set_pool_connections_idle(stats.available as i64);
    metrics::set_pool_connections_waiting(stats.waiting as i64);
    
    // Perform a health check on the pool
    match pool.get().await {
        Ok(conn) => {
            // Connection acquired successfully, perform a simple health check
            match conn.query("RETURN 1").await {
                Ok(_) => {
                    debug!("Pool health check passed");
                }
                Err(e) => {
                    error!("Pool health check failed: {}", e);
                    metrics::increment_pool_health_check_failures();
                }
            }
        }
        Err(e) => {
            error!("Failed to acquire connection from pool: {}", e);
            metrics::increment_pool_connection_errors();
        }
    }
}

/// Connection pool statistics (re-exported from pool module)
pub use crate::pool::PoolStats;