use clap::Parser;
use std::fmt;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Config {
    #[arg(long, env = "DB_URL", default_value = "ws://localhost:8000")]
    pub db_url: String,

    #[arg(long, env = "DB_USER", default_value = "root")]
    pub db_user: String,

    #[arg(long, env = "DB_PASS", default_value = "root")]
    pub db_pass: String,

    #[arg(long, env = "DB_NAMESPACE", default_value = "gitstars")]
    pub db_namespace: String,

    #[arg(long, env = "DB_DATABASE", default_value = "stars")]
    pub db_database: String,

    /// Embedding provider: "ollama", "openai", or "together"
    #[arg(long, env = "EMBEDDING_PROVIDER", default_value = "ollama")]
    pub embedding_provider: String,

    #[arg(long, env = "OLLAMA_URL", default_value = "http://localhost:11434")]
    pub ollama_url: String,

    #[arg(long, env = "OPENAI_API_KEY")]
    pub openai_api_key: Option<String>,

    #[arg(long, env = "TOGETHER_API_KEY")]
    pub together_api_key: Option<String>,

    #[arg(long, env = "EMBEDDING_MODEL", default_value = "nomic-embed-text")]
    pub embedding_model: String,

    #[arg(long, env = "BATCH_SIZE", default_value = "10")]
    pub batch_size: usize,

    #[arg(long, env = "POOL_SIZE", default_value = "10")]
    pub pool_size: usize,

    #[arg(long, env = "RETRY_ATTEMPTS", default_value = "3")]
    pub retry_attempts: u32,

    #[arg(long, env = "RETRY_DELAY_MS", default_value = "1000")]
    pub retry_delay_ms: u64,

    #[arg(long, env = "BATCH_DELAY_MS", default_value = "100")]
    pub batch_delay_ms: u64,

    #[arg(long, env = "MONITORING_PORT", default_value = "9090")]
    pub monitoring_port: Option<u16>,

    #[arg(long, env = "PARALLEL_WORKERS", default_value = "3")]
    pub parallel_workers: usize,

    #[arg(long, env = "TOKEN_LIMIT", default_value = "8000")]
    pub token_limit: usize,

    #[arg(long, env = "POOL_MAX_SIZE", default_value = "10")]
    pub pool_max_size: usize,

    #[arg(long, env = "POOL_TIMEOUT_SECS", default_value = "30")]
    pub pool_timeout_secs: u64,

    #[arg(long, env = "POOL_WAIT_TIMEOUT_SECS", default_value = "10")]
    pub pool_wait_timeout_secs: u64,

    #[arg(long, env = "POOL_CREATE_TIMEOUT_SECS", default_value = "30")]
    pub pool_create_timeout_secs: u64,

    #[arg(long, env = "POOL_RECYCLE_TIMEOUT_SECS", default_value = "30")]
    pub pool_recycle_timeout_secs: u64,
}

impl Config {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.embedding_provider == "openai" && self.openai_api_key.is_none() {
            anyhow::bail!("OpenAI API key is required when using OpenAI as embedding provider");
        }

        if self.embedding_provider == "together" && self.together_api_key.is_none() {
            anyhow::bail!("Together AI API key is required when using Together AI as embedding provider");
        }

        if self.batch_size == 0 {
            anyhow::bail!("Batch size must be greater than 0");
        }

        if self.pool_size == 0 {
            anyhow::bail!("Pool size must be greater than 0");
        }

        if self.pool_max_size == 0 {
            anyhow::bail!("Pool max size must be greater than 0");
        }

        if self.pool_max_size < self.pool_size {
            anyhow::bail!("Pool max size must be greater than or equal to pool size");
        }

        if self.parallel_workers == 0 {
            anyhow::bail!("Parallel workers must be greater than 0");
        }

        if self.token_limit == 0 {
            anyhow::bail!("Token limit must be greater than 0");
        }

        Ok(())
    }
}

impl fmt::Display for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Configuration:")?;
        writeln!(f, "  Database URL: {}", self.db_url)?;
        writeln!(f, "  Database: {}/{}", self.db_namespace, self.db_database)?;
        writeln!(f, "  Embedding Provider: {}", self.embedding_provider)?;
        writeln!(f, "  Embedding Model: {}", self.embedding_model)?;
        writeln!(f, "  Token Limit: {} characters", self.token_limit)?;
        writeln!(f, "  Batch Size: {}", self.batch_size)?;
        writeln!(f, "  Pool Size: {} (max: {})", self.pool_size, self.pool_max_size)?;
        writeln!(f, "  Pool Timeouts: wait={}s, create={}s, recycle={}s", 
            self.pool_wait_timeout_secs, 
            self.pool_create_timeout_secs, 
            self.pool_recycle_timeout_secs)?;
        Ok(())
    }
}