use crate::deduplication::DeduplicationManager;
use std::sync::Arc;
use tokio::time::{interval, Duration};
use tracing::{error, info};

/// Periodically clean up expired processing locks
pub async fn cleanup_locks_loop(
    deduplication: Arc<DeduplicationManager>,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) {
    let mut interval = interval(Duration::from_secs(300)); // Every 5 minutes

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                info!("Lock cleanup task received shutdown signal");
                break;
            }
            _ = interval.tick() => {
                match deduplication.cleanup_expired_locks().await {
                    Ok(_) => {
                        // Also log active locks count for monitoring
                        match deduplication.get_active_locks_count().await {
                            Ok(count) => {
                                info!(
                                    active_locks = count,
                                    "Cleaned up expired locks"
                                );
                            }
                            Err(e) => {
                                error!("Failed to get active locks count: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to cleanup expired locks: {}", e);
                    }
                }
            }
        }
    }
}