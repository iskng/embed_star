use governor::{Quota, RateLimiter as GovernorRateLimiter};
use governor::clock::{QuantaClock, QuantaInstant};
use governor::state::{InMemoryState, NotKeyed};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::num::NonZeroU32;
use crate::error::{EmbedError, Result};

type RateLimiterInstance = GovernorRateLimiter<NotKeyed, InMemoryState, QuantaClock, governor::middleware::NoOpMiddleware<QuantaInstant>>;

pub struct RateLimiterManager {
    limiters: Arc<RwLock<HashMap<String, Arc<RateLimiterInstance>>>>,
}

impl RateLimiterManager {
    pub fn new() -> Self {
        Self {
            limiters: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    pub async fn configure_provider(&self, provider: &str, requests_per_minute: u32) -> Result<()> {
        if requests_per_minute == 0 {
            return Ok(());
        }
        
        let quota = Quota::per_minute(NonZeroU32::new(requests_per_minute).unwrap());
        let limiter = Arc::new(GovernorRateLimiter::direct(quota));
        
        let mut limiters = self.limiters.write().await;
        limiters.insert(provider.to_string(), limiter);
        
        Ok(())
    }
    
    pub async fn check_rate_limit(&self, provider: &str) -> Result<()> {
        let limiters = self.limiters.read().await;
        
        if let Some(limiter) = limiters.get(provider) {
            match limiter.check() {
                Ok(_) => Ok(()),
                Err(_) => {
                    crate::metrics::record_rate_limit(provider);
                    Err(EmbedError::RateLimitExceeded {
                        provider: provider.to_string(),
                    })
                }
            }
        } else {
            // No rate limit configured for this provider
            Ok(())
        }
    }
    
    pub async fn wait_for_permit(&self, provider: &str) -> Result<()> {
        let limiters = self.limiters.read().await;
        
        if let Some(limiter) = limiters.get(provider) {
            limiter.until_ready().await;
            Ok(())
        } else {
            Ok(())
        }
    }
}

impl Default for RateLimiterManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_rate_limiter() {
        let manager = RateLimiterManager::new();
        
        // Configure 2 requests per minute (for testing)
        manager.configure_provider("test", 2).await.unwrap();
        
        // First two requests should succeed
        assert!(manager.check_rate_limit("test").await.is_ok());
        assert!(manager.check_rate_limit("test").await.is_ok());
        
        // Third request should fail
        assert!(matches!(
            manager.check_rate_limit("test").await,
            Err(EmbedError::RateLimitExceeded { .. })
        ));
    }
}