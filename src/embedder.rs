use crate::config::Config;
use crate::embedding_validation::{EmbeddingValidator, together_e5_validator};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>>;
    fn model_name(&self) -> &str;
}

pub struct OllamaEmbedder {
    client: ollama_rs::Ollama,
    model: String,
}

impl OllamaEmbedder {
    pub fn new(url: &str, model: String) -> Result<Self> {
        let client = ollama_rs::Ollama::new(url.to_string(), 11434);
        Ok(Self { client, model })
    }
}

#[async_trait]
impl EmbeddingProvider for OllamaEmbedder {
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
        
        use ollama_rs::generation::embeddings::request::EmbeddingsInput;
        
        let request = GenerateEmbeddingsRequest::new(
            self.model.clone(), 
            EmbeddingsInput::Single(text.to_string())
        );

        let response = self
            .client
            .generate_embeddings(request)
            .await
            .map_err(|e| anyhow::anyhow!("Ollama embedding generation failed: {}", e))?;

        // ollama-rs returns Vec<Vec<f32>>, we need to get the first embedding
        response.embeddings
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No embeddings returned from Ollama"))
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

pub struct OpenAIEmbedder {
    client: async_openai::Client<async_openai::config::OpenAIConfig>,
    model: String,
}

impl OpenAIEmbedder {
    pub fn new(api_key: &str, model: String) -> Result<Self> {
        let config = async_openai::config::OpenAIConfig::new().with_api_key(api_key);
        let client = async_openai::Client::with_config(config);
        Ok(Self { client, model })
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAIEmbedder {
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        use async_openai::types::{CreateEmbeddingRequest, EmbeddingInput};

        let request = CreateEmbeddingRequest {
            model: self.model.clone(),
            input: EmbeddingInput::String(text.to_string()),
            encoding_format: None,
            user: None,
            dimensions: None,
        };

        let response = self
            .client
            .embeddings()
            .create(request)
            .await
            .map_err(|e| anyhow::anyhow!("OpenAI embedding generation failed: {}", e))?;

        if let Some(embedding) = response.data.first() {
            Ok(embedding.embedding.clone())
        } else {
            Err(anyhow::anyhow!("No embedding returned from OpenAI"))
        }
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

pub struct TogetherAIEmbedder {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

impl TogetherAIEmbedder {
    pub fn new(api_key: &str, model: String) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Self {
            client,
            api_key: api_key.to_string(),
            model,
        })
    }
}

#[async_trait]
impl EmbeddingProvider for TogetherAIEmbedder {
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        #[derive(Serialize)]
        struct TogetherRequest {
            model: String,
            input: String,
        }

        #[derive(Deserialize)]
        struct TogetherResponse {
            data: Vec<EmbeddingData>,
        }

        #[derive(Deserialize)]
        struct EmbeddingData {
            embedding: Vec<f32>,
        }

        let request_body = TogetherRequest {
            model: self.model.clone(),
            input: text.to_string(),
        };

        let response = self
            .client
            .post("https://api.together.xyz/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Together AI request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Together AI API error ({}): {}",
                status,
                error_text
            ));
        }

        let together_response: TogetherResponse = response
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse Together AI response: {}", e))?;

        together_response
            .data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .ok_or_else(|| anyhow::anyhow!("No embeddings returned from Together AI"))
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

pub struct Embedder {
    provider: Box<dyn EmbeddingProvider>,
    retry_attempts: u32,
    retry_delay_ms: u64,
    token_limit: usize,
    validator: Option<EmbeddingValidator>,
}

impl Embedder {
    pub fn new(config: Arc<Config>) -> Result<Self> {
        let provider: Box<dyn EmbeddingProvider> = match config.embedding_provider.as_str() {
            "ollama" => {
                info!(
                    "Using Ollama embedder with model: {}",
                    config.embedding_model
                );
                Box::new(OllamaEmbedder::new(
                    &config.ollama_url,
                    config.embedding_model.clone(),
                )?)
            }
            "openai" => {
                let api_key = config
                    .openai_api_key
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("OpenAI API key not provided"))?;
                info!(
                    "Using OpenAI embedder with model: {}",
                    config.embedding_model
                );
                Box::new(OpenAIEmbedder::new(api_key, config.embedding_model.clone())?)
            }
            "together" | "togetherai" => {
                let api_key = config
                    .together_api_key
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Together AI API key not provided"))?;
                info!(
                    "Using Together AI embedder with model: {}",
                    config.embedding_model
                );
                Box::new(TogetherAIEmbedder::new(api_key, config.embedding_model.clone())?)
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "Unknown embedding provider: {}",
                    config.embedding_provider
                ))
            }
        };

        // Set up validator based on the model
        let validator = match config.embedding_model.as_str() {
            "intfloat/multilingual-e5-large-instruct" => Some(together_e5_validator()),
            _ => None, // No validation for other models yet
        };

        Ok(Self {
            provider,
            retry_attempts: config.retry_attempts,
            retry_delay_ms: config.retry_delay_ms,
            token_limit: config.token_limit,
            validator,
        })
    }

    fn truncate_text(&self, text: &str) -> String {
        if text.len() <= self.token_limit {
            return text.to_string();
        }

        // Truncate to token limit and add ellipsis
        let truncated = text.chars().take(self.token_limit - 3).collect::<String>();
        info!(
            "Text truncated from {} to {} characters (token limit: {})",
            text.len(),
            truncated.len() + 3,
            self.token_limit
        );
        format!("{}...", truncated)
    }

    pub async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        let truncated_text = self.truncate_text(text);
        let mut attempts = 0;

        loop {
            attempts += 1;
            match self.provider.generate_embedding(&truncated_text).await {
                Ok(embedding) => {
                    // Validate the embedding if validator is configured
                    if let Some(validator) = &self.validator {
                        match validator.validate(&embedding, &format!("{}:{}", self.model_name(), text.chars().take(50).collect::<String>())) {
                            Ok(_) => {
                                crate::metrics::record_embedding_validation(self.model_name(), true);
                            }
                            Err(e) => {
                                crate::metrics::record_embedding_validation(self.model_name(), false);
                                error!("Embedding validation failed: {}", e);
                                // Convert to retryable error so we can try again
                                if attempts < self.retry_attempts {
                                    warn!("Retrying due to validation failure...");
                                    tokio::time::sleep(tokio::time::Duration::from_millis(self.retry_delay_ms)).await;
                                    continue;
                                }
                                return Err(anyhow::anyhow!("Embedding validation failed: {}", e));
                            }
                        }
                    }
                    
                    debug!(
                        "Generated embedding with {} dimensions",
                        embedding.len()
                    );
                    return Ok(embedding);
                }
                Err(e) => {
                    if attempts >= self.retry_attempts {
                        error!(
                            "Failed to generate embedding after {} attempts: {}",
                            attempts, e
                        );
                        return Err(e);
                    }
                    warn!(
                        "Embedding generation attempt {} failed: {}. Retrying...",
                        attempts, e
                    );
                    tokio::time::sleep(tokio::time::Duration::from_millis(self.retry_delay_ms))
                        .await;
                }
            }
        }
    }

    pub fn model_name(&self) -> &str {
        self.provider.model_name()
    }

    pub fn set_validator(&mut self, validator: Option<EmbeddingValidator>) {
        self.validator = validator;
    }

    pub fn enable_validation(&mut self) {
        if self.validator.is_none() {
            // Use default validator
            self.validator = Some(EmbeddingValidator::default());
        }
    }

    pub fn disable_validation(&mut self) {
        self.validator = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_truncation() {
        // Create a mock config
        let config = Config {
            db_url: "ws://localhost:8000".to_string(),
            db_user: "root".to_string(),
            db_pass: "root".to_string(),
            db_namespace: "test".to_string(),
            db_database: "test".to_string(),
            embedding_provider: "ollama".to_string(),
            ollama_url: "http://localhost:11434".to_string(),
            openai_api_key: None,
            together_api_key: None,
            embedding_model: "test-model".to_string(),
            batch_size: 10,
            batch_delay_ms: 100,
            pool_size: 10,
            retry_attempts: 3,
            retry_delay_ms: 1000,
            monitoring_port: None,
            parallel_workers: 1,
            token_limit: 100, // Small limit for testing
            pool_max_size: 10,
            pool_timeout_secs: 30,
            pool_wait_timeout_secs: 10,
            pool_create_timeout_secs: 30,
            pool_recycle_timeout_secs: 30,
        };

        // Create embedder (will fail to connect but that's OK for this test)
        let embedder = Embedder::new(Arc::new(config)).unwrap();

        // Test short text (should not be truncated)
        let short_text = "This is a short text";
        let result = embedder.truncate_text(short_text);
        assert_eq!(result, short_text);

        // Test long text (should be truncated)
        let long_text = "a".repeat(200); // 200 characters
        let result = embedder.truncate_text(&long_text);
        assert_eq!(result.len(), 100); // 97 chars + "..."
        assert!(result.ends_with("..."));
        assert_eq!(&result[..97], &long_text[..97]);

        // Test exact limit
        let exact_text = "b".repeat(100);
        let result = embedder.truncate_text(&exact_text);
        assert_eq!(result, exact_text); // Should not be truncated
    }
}