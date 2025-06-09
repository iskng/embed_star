use crate::config::Config;
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

        Ok(Self {
            provider,
            retry_attempts: config.retry_attempts,
            retry_delay_ms: config.retry_delay_ms,
        })
    }

    pub async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        let mut attempts = 0;

        loop {
            attempts += 1;
            match self.provider.generate_embedding(text).await {
                Ok(embedding) => {
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
}