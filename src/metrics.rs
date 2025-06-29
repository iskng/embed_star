use prometheus::{
    register_counter_vec, register_histogram_vec, register_int_gauge, register_int_gauge_vec,
    CounterVec, HistogramVec, IntGauge, IntGaugeVec, Registry,
};
use std::sync::OnceLock;

pub struct Metrics {
    pub embeddings_total: CounterVec,
    pub embeddings_errors: CounterVec,
    pub embedding_duration: HistogramVec,
    pub repos_pending: IntGauge,
    pub repos_processed: IntGauge,
    pub provider_requests: CounterVec,
    pub rate_limits: CounterVec,
    pub active_connections: IntGaugeVec,
    pub circuit_breaker_state: IntGaugeVec,
    pub retry_attempts: CounterVec,
    pub pool_connections_active: IntGauge,
    pub pool_connections_idle: IntGauge,
    pub pool_connections_waiting: IntGauge,
    pub pool_connections_created: CounterVec,
    pub pool_connections_recycled: CounterVec,
    pub pool_connection_errors: CounterVec,
    pub pool_health_check_failures: CounterVec,
    pub embedding_validations: CounterVec,
}

static METRICS: OnceLock<Metrics> = OnceLock::new();

impl Metrics {
    pub fn new(_registry: &Registry) -> prometheus::Result<Self> {
        Ok(Self {
            embeddings_total: register_counter_vec!(
                prometheus::opts!("embed_star_embeddings_total", "Total number of embeddings generated"),
                &["provider", "model"]
            )?,
            embeddings_errors: register_counter_vec!(
                prometheus::opts!("embed_star_embeddings_errors_total", "Total number of embedding errors"),
                &["provider", "error_type"]
            )?,
            embedding_duration: {
                let opts = prometheus::HistogramOpts::new(
                    "embed_star_embedding_duration_seconds",
                    "Time taken to generate embeddings"
                ).buckets(vec![0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]);
                register_histogram_vec!(opts, &["provider", "model"])?
            },
            repos_pending: register_int_gauge!(
                prometheus::opts!("embed_star_repos_pending", "Number of repos pending embedding generation")
            )?,
            repos_processed: register_int_gauge!(
                prometheus::opts!("embed_star_repos_processed", "Total number of repos processed")
            )?,
            provider_requests: register_counter_vec!(
                prometheus::opts!("embed_star_provider_requests_total", "Total requests to embedding providers"),
                &["provider", "status"]
            )?,
            rate_limits: register_counter_vec!(
                prometheus::opts!("embed_star_rate_limits_total", "Total number of rate limit hits"),
                &["provider"]
            )?,
            active_connections: register_int_gauge_vec!(
                prometheus::opts!("embed_star_active_connections", "Number of active connections"),
                &["type"]
            )?,
            circuit_breaker_state: register_int_gauge_vec!(
                prometheus::opts!("embed_star_circuit_breaker_state", "Circuit breaker state (0=closed, 1=open, 2=half-open)"),
                &["service"]
            )?,
            retry_attempts: register_counter_vec!(
                prometheus::opts!("embed_star_retry_attempts_total", "Total retry attempts"),
                &["operation"]
            )?,
            pool_connections_active: register_int_gauge!(
                prometheus::opts!("embed_star_pool_connections_active", "Number of active pool connections")
            )?,
            pool_connections_idle: register_int_gauge!(
                prometheus::opts!("embed_star_pool_connections_idle", "Number of idle pool connections")
            )?,
            pool_connections_waiting: register_int_gauge!(
                prometheus::opts!("embed_star_pool_connections_waiting", "Number of requests waiting for a connection")
            )?,
            pool_connections_created: register_counter_vec!(
                prometheus::opts!("embed_star_pool_connections_created_total", "Total pool connections created"),
                &["pool"]
            )?,
            pool_connections_recycled: register_counter_vec!(
                prometheus::opts!("embed_star_pool_connections_recycled_total", "Total pool connections recycled"),
                &["pool"]
            )?,
            pool_connection_errors: register_counter_vec!(
                prometheus::opts!("embed_star_pool_connection_errors_total", "Total pool connection errors"),
                &["pool", "error_type"]
            )?,
            pool_health_check_failures: register_counter_vec!(
                prometheus::opts!("embed_star_pool_health_check_failures_total", "Total pool health check failures"),
                &["pool"]
            )?,
            embedding_validations: register_counter_vec!(
                prometheus::opts!("embed_star_embedding_validations_total", "Total embedding validation attempts"),
                &["model", "status"]
            )?,
        })
    }
    
    pub fn register(registry: &Registry) -> prometheus::Result<()> {
        let metrics = Self::new(registry)?;
        
        registry.register(Box::new(metrics.embeddings_total.clone()))?;
        registry.register(Box::new(metrics.embeddings_errors.clone()))?;
        registry.register(Box::new(metrics.embedding_duration.clone()))?;
        registry.register(Box::new(metrics.repos_pending.clone()))?;
        registry.register(Box::new(metrics.repos_processed.clone()))?;
        registry.register(Box::new(metrics.provider_requests.clone()))?;
        registry.register(Box::new(metrics.rate_limits.clone()))?;
        registry.register(Box::new(metrics.active_connections.clone()))?;
        registry.register(Box::new(metrics.circuit_breaker_state.clone()))?;
        registry.register(Box::new(metrics.retry_attempts.clone()))?;
        registry.register(Box::new(metrics.pool_connections_active.clone()))?;
        registry.register(Box::new(metrics.pool_connections_idle.clone()))?;
        registry.register(Box::new(metrics.pool_connections_waiting.clone()))?;
        registry.register(Box::new(metrics.pool_connections_created.clone()))?;
        registry.register(Box::new(metrics.pool_connections_recycled.clone()))?;
        registry.register(Box::new(metrics.pool_connection_errors.clone()))?;
        registry.register(Box::new(metrics.pool_health_check_failures.clone()))?;
        registry.register(Box::new(metrics.embedding_validations.clone()))?;
        
        METRICS.set(metrics).map_err(|_| prometheus::Error::Msg("Metrics already initialized".to_string()))?;
        Ok(())
    }
    
    pub fn get() -> &'static Metrics {
        METRICS.get().expect("Metrics not initialized")
    }
}

pub fn record_embedding_generated(provider: &str, model: &str, duration: f64) {
    let metrics = Metrics::get();
    metrics.embeddings_total.with_label_values(&[provider, model]).inc();
    metrics.embedding_duration.with_label_values(&[provider, model]).observe(duration);
    metrics.repos_processed.inc();
}

pub fn record_embedding_error(provider: &str, error_type: &str) {
    let metrics = Metrics::get();
    metrics.embeddings_errors.with_label_values(&[provider, error_type]).inc();
}

pub fn record_provider_request(provider: &str, success: bool) {
    let metrics = Metrics::get();
    let status = if success { "success" } else { "failure" };
    metrics.provider_requests.with_label_values(&[provider, status]).inc();
}

pub fn record_rate_limit(provider: &str) {
    let metrics = Metrics::get();
    metrics.rate_limits.with_label_values(&[provider]).inc();
}

pub fn set_pending_repos(count: i64) {
    let metrics = Metrics::get();
    metrics.repos_pending.set(count);
}

pub fn update_active_connections(conn_type: &str, delta: i64) {
    let metrics = Metrics::get();
    metrics.active_connections.with_label_values(&[conn_type]).add(delta);
}

pub fn record_circuit_breaker_state(service: &str, state: &str) {
    let metrics = Metrics::get();
    let value = match state {
        "closed" => 0,
        "open" => 1,
        "half_open" => 2,
        _ => 0,
    };
    metrics.circuit_breaker_state.with_label_values(&[service]).set(value);
}

pub fn record_retry(operation: &str) {
    let metrics = Metrics::get();
    metrics.retry_attempts.with_label_values(&[operation]).inc();
}

pub fn set_pool_connections_active(count: i64) {
    let metrics = Metrics::get();
    metrics.pool_connections_active.set(count);
}

pub fn set_pool_connections_idle(count: i64) {
    let metrics = Metrics::get();
    metrics.pool_connections_idle.set(count);
}

pub fn set_pool_connections_waiting(count: i64) {
    let metrics = Metrics::get();
    metrics.pool_connections_waiting.set(count);
}

pub fn increment_pool_connections_created() {
    let metrics = Metrics::get();
    metrics.pool_connections_created.with_label_values(&["surrealdb"]).inc();
}

pub fn increment_pool_connections_recycled() {
    let metrics = Metrics::get();
    metrics.pool_connections_recycled.with_label_values(&["surrealdb"]).inc();
}

pub fn increment_pool_connection_errors() {
    let metrics = Metrics::get();
    metrics.pool_connection_errors.with_label_values(&["surrealdb", "create"]).inc();
}

pub fn increment_pool_health_check_failures() {
    let metrics = Metrics::get();
    metrics.pool_health_check_failures.with_label_values(&["surrealdb"]).inc();
}

pub fn record_embedding_validation(model: &str, success: bool) {
    let metrics = Metrics::get();
    let status = if success { "pass" } else { "fail" };
    metrics.embedding_validations.with_label_values(&[model, status]).inc();
}