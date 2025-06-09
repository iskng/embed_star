use std::time::Duration;
use tracing::{debug, warn};
use crate::error::{EmbedError, Result};

pub struct RetryConfig {
    pub max_retries: u32,
    pub initial_interval: Duration,
    pub max_interval: Duration,
    pub multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_interval: Duration::from_millis(100),
            max_interval: Duration::from_secs(10),
            multiplier: 2.0,
        }
    }
}

pub async fn with_retry<F, Fut, T>(
    operation_name: &str,
    config: &RetryConfig,
    mut operation: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    use backoff::{backoff::Backoff, ExponentialBackoff};
    
    let mut backoff = ExponentialBackoff {
        initial_interval: config.initial_interval,
        max_interval: config.max_interval,
        multiplier: config.multiplier,
        max_elapsed_time: None,
        ..Default::default()
    };
    
    let mut retry_count = 0;
    let mut last_error: Option<EmbedError> = None;
    
    loop {
        match operation().await {
            Ok(result) => {
                if retry_count > 0 {
                    debug!(
                        "Operation '{}' succeeded after {} retries",
                        operation_name, retry_count
                    );
                }
                return Ok(result);
            }
            Err(error) => {
                if !error.is_retryable() || retry_count >= config.max_retries {
                    warn!(
                        "Operation '{}' failed after {} retries: {:?}",
                        operation_name, retry_count, error
                    );
                    return Err(error);
                }
                
                retry_count += 1;
                last_error = Some(error);
                
                if let Some(duration) = backoff.next_backoff() {
                    warn!(
                        "Operation '{}' failed (attempt {}/{}), retrying in {:?}",
                        operation_name, retry_count, config.max_retries, duration
                    );
                    tokio::time::sleep(duration).await;
                } else {
                    break;
                }
            }
        }
    }
    
    Err(last_error.unwrap_or_else(|| {
        EmbedError::Internal(anyhow::anyhow!(
            "Retry logic failed for operation '{}'",
            operation_name
        ))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    
    #[tokio::test]
    async fn test_retry_success_after_failures() {
        let attempts = Arc::new(AtomicU32::new(0));
        let attempts_clone = attempts.clone();
        
        let config = RetryConfig {
            max_retries: 3,
            initial_interval: Duration::from_millis(10),
            max_interval: Duration::from_millis(100),
            multiplier: 2.0,
        };
        
        let result = with_retry("test_operation", &config, || {
            let attempts = attempts_clone.clone();
            async move {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                if attempt < 2 {
                    Err(EmbedError::ServiceUnavailable("test error".to_string()))
                } else {
                    Ok(42)
                }
            }
        })
        .await;
        
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }
    
    #[tokio::test]
    async fn test_retry_non_retryable_error() {
        let attempts = Arc::new(AtomicU32::new(0));
        let attempts_clone = attempts.clone();
        
        let config = RetryConfig::default();
        
        let result: Result<()> = with_retry("test_operation", &config, || {
            let attempts = attempts_clone.clone();
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                Err(EmbedError::Configuration("non-retryable".to_string()))
            }
        })
        .await;
        
        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 1); // Should not retry
    }
}