use parking_lot::RwLock;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tracing::{debug, info, warn};

/// Circuit breaker states
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CircuitState {
    /// Circuit is closed, requests flow normally
    Closed,
    /// Circuit is open, requests are rejected
    Open,
    /// Circuit is half-open, limited requests allowed to test recovery
    HalfOpen,
}

/// Circuit breaker statistics
#[derive(Debug, Clone)]
pub struct CircuitStats {
    pub total_requests: u64,
    pub failed_requests: u64,
    pub successful_requests: u64,
    pub consecutive_failures: u32,
    pub last_failure_time: Option<Instant>,
    pub state_changes: u64,
}

impl Default for CircuitStats {
    fn default() -> Self {
        Self {
            total_requests: 0,
            failed_requests: 0,
            successful_requests: 0,
            consecutive_failures: 0,
            last_failure_time: None,
            state_changes: 0,
        }
    }
}

/// Configuration for a circuit breaker
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures before opening the circuit
    pub failure_threshold: u32,
    /// Duration to keep the circuit open before trying half-open
    pub timeout_duration: Duration,
    /// Number of successful requests in half-open state before closing
    pub success_threshold: u32,
    /// Failure rate threshold (0.0 to 1.0) for opening the circuit
    pub failure_rate_threshold: f64,
    /// Minimum number of requests before failure rate is considered
    pub min_requests: u64,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            timeout_duration: Duration::from_secs(60),
            success_threshold: 3,
            failure_rate_threshold: 0.5,
            min_requests: 10,
        }
    }
}

/// Individual circuit breaker instance
struct CircuitBreaker {
    state: CircuitState,
    stats: CircuitStats,
    config: CircuitBreakerConfig,
    last_state_change: Instant,
    half_open_successes: u32,
}

impl CircuitBreaker {
    fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            state: CircuitState::Closed,
            stats: CircuitStats::default(),
            config,
            last_state_change: Instant::now(),
            half_open_successes: 0,
        }
    }

    fn should_allow_request(&mut self) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // Check if timeout has passed
                if self.last_state_change.elapsed() >= self.config.timeout_duration {
                    self.transition_to(CircuitState::HalfOpen);
                    true
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => true,
        }
    }

    fn record_success(&mut self) {
        self.stats.total_requests += 1;
        self.stats.successful_requests += 1;
        self.stats.consecutive_failures = 0;

        match self.state {
            CircuitState::HalfOpen => {
                self.half_open_successes += 1;
                if self.half_open_successes >= self.config.success_threshold {
                    self.transition_to(CircuitState::Closed);
                }
            }
            _ => {}
        }
    }

    fn record_failure(&mut self) {
        self.stats.total_requests += 1;
        self.stats.failed_requests += 1;
        self.stats.consecutive_failures += 1;
        self.stats.last_failure_time = Some(Instant::now());

        match self.state {
            CircuitState::Closed => {
                // Check if we should open the circuit
                if self.stats.consecutive_failures >= self.config.failure_threshold {
                    self.transition_to(CircuitState::Open);
                } else if self.stats.total_requests >= self.config.min_requests {
                    let failure_rate = self.stats.failed_requests as f64 / self.stats.total_requests as f64;
                    if failure_rate >= self.config.failure_rate_threshold {
                        self.transition_to(CircuitState::Open);
                    }
                }
            }
            CircuitState::HalfOpen => {
                // Any failure in half-open state reopens the circuit
                self.transition_to(CircuitState::Open);
            }
            CircuitState::Open => {}
        }
    }

    fn transition_to(&mut self, new_state: CircuitState) {
        if self.state != new_state {
            info!(
                "Circuit breaker state transition: {:?} -> {:?}",
                self.state, new_state
            );
            self.state = new_state;
            self.stats.state_changes += 1;
            self.last_state_change = Instant::now();
            
            if new_state == CircuitState::HalfOpen {
                self.half_open_successes = 0;
            }
        }
    }
}

/// Manages multiple circuit breakers for different services
pub struct CircuitBreakerManager {
    breakers: Arc<RwLock<HashMap<String, CircuitBreaker>>>,
    default_config: CircuitBreakerConfig,
}

impl CircuitBreakerManager {
    pub fn new() -> Self {
        Self {
            breakers: Arc::new(RwLock::new(HashMap::new())),
            default_config: CircuitBreakerConfig::default(),
        }
    }

    /// Configure a specific service with custom settings
    pub fn configure_service(&self, service: &str, config: CircuitBreakerConfig) {
        let mut breakers = self.breakers.write();
        breakers.insert(service.to_string(), CircuitBreaker::new(config));
        info!("Configured circuit breaker for service: {}", service);
    }

    /// Check if a request should be allowed for a service
    pub fn should_allow_request(&self, service: &str) -> bool {
        let mut breakers = self.breakers.write();
        let breaker = breakers
            .entry(service.to_string())
            .or_insert_with(|| CircuitBreaker::new(self.default_config.clone()));

        let allowed = breaker.should_allow_request();
        
        if !allowed {
            warn!("Circuit breaker OPEN for service: {}", service);
            crate::metrics::record_circuit_breaker_state(service, "open");
        } else {
            let state_str = match breaker.state {
                CircuitState::Closed => "closed",
                CircuitState::Open => "open",
                CircuitState::HalfOpen => "half_open",
            };
            crate::metrics::record_circuit_breaker_state(service, state_str);
        }

        allowed
    }

    /// Record a successful request
    pub fn record_success(&self, service: &str) {
        let mut breakers = self.breakers.write();
        if let Some(breaker) = breakers.get_mut(service) {
            breaker.record_success();
            debug!("Recorded success for service: {}", service);
        }
    }

    /// Record a failed request
    pub fn record_failure(&self, service: &str) {
        let mut breakers = self.breakers.write();
        if let Some(breaker) = breakers.get_mut(service) {
            breaker.record_failure();
            warn!(
                "Recorded failure for service: {} (consecutive failures: {})",
                service, breaker.stats.consecutive_failures
            );
        }
    }

    /// Get statistics for a service
    pub fn get_stats(&self, service: &str) -> Option<CircuitStats> {
        let breakers = self.breakers.read();
        breakers.get(service).map(|b| b.stats.clone())
    }

    /// Get current state for a service
    pub fn get_state(&self, service: &str) -> Option<CircuitState> {
        let breakers = self.breakers.read();
        breakers.get(service).map(|b| b.state)
    }

    /// Get all services and their states
    pub fn get_all_states(&self) -> HashMap<String, CircuitState> {
        let breakers = self.breakers.read();
        breakers
            .iter()
            .map(|(service, breaker)| (service.clone(), breaker.state))
            .collect()
    }

    /// Reset a circuit breaker for a service
    pub fn reset(&self, service: &str) {
        let mut breakers = self.breakers.write();
        if let Some(breaker) = breakers.get_mut(service) {
            breaker.state = CircuitState::Closed;
            breaker.stats.consecutive_failures = 0;
            breaker.half_open_successes = 0;
            breaker.last_state_change = Instant::now();
            info!("Reset circuit breaker for service: {}", service);
        }
    }
}

impl Default for CircuitBreakerManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper macro to execute code with circuit breaker protection
#[macro_export]
macro_rules! with_circuit_breaker {
    ($manager:expr, $service:expr, $operation:expr) => {{
        if !$manager.should_allow_request($service) {
            Err($crate::error::EmbedError::ServiceUnavailable(format!(
                "Circuit breaker open for service: {}",
                $service
            )))
        } else {
            match $operation {
                Ok(result) => {
                    $manager.record_success($service);
                    Ok(result)
                }
                Err(e) => {
                    $manager.record_failure($service);
                    Err(e)
                }
            }
        }
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circuit_breaker_opens_on_threshold() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            ..Default::default()
        };
        let mut breaker = CircuitBreaker::new(config);

        assert_eq!(breaker.state, CircuitState::Closed);

        // Record failures
        for _ in 0..3 {
            assert!(breaker.should_allow_request());
            breaker.record_failure();
        }

        // Circuit should now be open
        assert_eq!(breaker.state, CircuitState::Open);
        assert!(!breaker.should_allow_request());
    }

    #[test]
    fn test_circuit_breaker_half_open_recovery() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            timeout_duration: Duration::from_millis(100),
            success_threshold: 2,
            ..Default::default()
        };
        let mut breaker = CircuitBreaker::new(config);

        // Open the circuit
        breaker.record_failure();
        assert_eq!(breaker.state, CircuitState::Open);

        // Wait for timeout
        std::thread::sleep(Duration::from_millis(150));

        // Should transition to half-open
        assert!(breaker.should_allow_request());
        assert_eq!(breaker.state, CircuitState::HalfOpen);

        // Record successes to close the circuit
        breaker.record_success();
        assert_eq!(breaker.state, CircuitState::HalfOpen);
        
        breaker.record_success();
        assert_eq!(breaker.state, CircuitState::Closed);
    }
}