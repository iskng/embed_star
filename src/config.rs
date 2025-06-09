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
}

impl Config {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.embedding_provider == "openai" && self.openai_api_key.is_none() {
            anyhow::bail!("OpenAI API key is required when using OpenAI as embedding provider");
        }

        if (self.embedding_provider == "together" || self.embedding_provider == "togetherai") 
            && self.together_api_key.is_none() {
            anyhow::bail!("Together AI API key is required when using Together AI as embedding provider");
        }

        if self.batch_size == 0 {
            anyhow::bail!("Batch size must be greater than 0");
        }

        if self.pool_size == 0 {
            anyhow::bail!("Pool size must be greater than 0");
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
        writeln!(f, "  Batch Size: {}", self.batch_size)?;
        writeln!(f, "  Pool Size: {}", self.pool_size)?;
        Ok(())
    }
}