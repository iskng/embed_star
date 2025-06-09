use thiserror::Error;

#[derive(Error, Debug)]
pub enum EmbedError {
    #[error("Database error: {0}")]
    Database(#[from] surrealdb::Error),
    
    #[error("Embedding provider error: {0}")]
    EmbeddingProvider(String),
    
    #[error("Configuration error: {0}")]
    Configuration(String),
    
    #[error("HTTP request error: {0}")]
    Http(#[from] reqwest::Error),
    
    #[error("Rate limit exceeded for {provider}")]
    RateLimitExceeded { provider: String },
    
    #[error("Invalid embedding dimension: expected {expected}, got {actual}")]
    InvalidDimension { expected: usize, actual: usize },
    
    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),
    
    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, EmbedError>;

impl EmbedError {
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            EmbedError::Database(_)
                | EmbedError::Http(_)
                | EmbedError::RateLimitExceeded { .. }
                | EmbedError::ServiceUnavailable(_)
        )
    }
    
    pub fn error_code(&self) -> &'static str {
        match self {
            EmbedError::Database(_) => "DATABASE_ERROR",
            EmbedError::EmbeddingProvider(_) => "EMBEDDING_ERROR",
            EmbedError::Configuration(_) => "CONFIG_ERROR",
            EmbedError::Http(_) => "HTTP_ERROR",
            EmbedError::RateLimitExceeded { .. } => "RATE_LIMIT",
            EmbedError::InvalidDimension { .. } => "INVALID_DIMENSION",
            EmbedError::ServiceUnavailable(_) => "SERVICE_UNAVAILABLE",
            EmbedError::Internal(_) => "INTERNAL_ERROR",
        }
    }
}