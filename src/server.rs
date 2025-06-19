use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use prometheus::{Encoder, Registry, TextEncoder};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use crate::pool::{Pool, PoolExt};

#[derive(Clone)]
pub struct AppState {
    pub db_pool: Pool,
    pub registry: Arc<Registry>,
}

#[derive(Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub database: DatabaseHealth,
    pub embedding_providers: Vec<ProviderHealth>,
}

#[derive(Serialize, Deserialize)]
pub struct DatabaseHealth {
    pub connected: bool,
    pub latency_ms: Option<u64>,
    pub pool_stats: Option<PoolStats>,
}

#[derive(Serialize, Deserialize)]
pub struct PoolStats {
    pub size: usize,
    pub available: usize,
    pub waiting: usize,
    pub max_size: usize,
}

#[derive(Serialize, Deserialize)]
pub struct ProviderHealth {
    pub name: String,
    pub available: bool,
    pub latency_ms: Option<u64>,
}


pub async fn health_check(State(state): State<AppState>) -> Result<Json<HealthResponse>, StatusCode> {
    // Check database health
    let db_start = std::time::Instant::now();
    let db_connected = match state.db_pool.get().await {
        Ok(conn) => {
            // Perform a simple health check query
            match conn.query("SELECT 1 as health_check").await {
                Ok(_) => true,
                Err(_) => false,
            }
        }
        Err(_) => false,
    };
    let db_latency = db_start.elapsed().as_millis() as u64;

    // Get pool statistics
    let pool_stats = state.db_pool.stats();
    
    let health = HealthResponse {
        status: if db_connected { "healthy".to_string() } else { "unhealthy".to_string() },
        version: env!("CARGO_PKG_VERSION").to_string(),
        database: DatabaseHealth {
            connected: db_connected,
            latency_ms: Some(db_latency),
            pool_stats: Some(PoolStats {
                size: pool_stats.size,
                available: pool_stats.available,
                waiting: pool_stats.waiting,
                max_size: pool_stats.max_size,
            }),
        },
        embedding_providers: vec![], // TODO: Check provider health
    };
    
    if db_connected {
        Ok(Json(health))
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}

pub async fn metrics_handler(State(state): State<AppState>) -> Result<Response, StatusCode> {
    let encoder = TextEncoder::new();
    let metric_families = state.registry.gather();
    let mut buffer = Vec::new();
    
    encoder
        .encode(&metric_families, &mut buffer)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", encoder.format_type())
        .body(buffer.into())
        .unwrap())
}

pub async fn liveness_check() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "alive",
        "timestamp": chrono::Utc::now()
    }))
}

pub fn create_monitoring_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/metrics", get(metrics_handler))
        .route("/livez", get(liveness_check))
        .with_state(state)
}

pub async fn run_monitoring_server(addr: &str, state: AppState) -> anyhow::Result<()> {
    let app = create_monitoring_router(state);
    
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Monitoring server listening on {}", addr);
    
    axum::serve(listener, app).await?;
    Ok(())
}