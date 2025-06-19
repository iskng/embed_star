use embed_star::{
    config::Config,
    embedder::Embedder,
    embedding_validation::together_e5_validator,
    metrics::Metrics,
};
use prometheus::Registry;
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("embed_star=debug".parse()?)
        )
        .init();

    info!("Starting embedding validation test");

    // Initialize metrics
    let registry = Registry::new();
    Metrics::register(&registry)?;
    info!("Metrics initialized");

    // Load environment variables
    dotenv::dotenv().ok();

    // Create config
    let config = Config {
        db_url: "ws://localhost:8000".to_string(),
        db_user: "root".to_string(),
        db_pass: "root".to_string(),
        db_namespace: "test".to_string(),
        db_database: "test".to_string(),
        embedding_provider: "together".to_string(),
        ollama_url: "http://localhost:11434".to_string(),
        openai_api_key: None,
        together_api_key: std::env::var("TOGETHER_API_KEY").ok(),
        embedding_model: "intfloat/multilingual-e5-large-instruct".to_string(),
        batch_size: 10,
        batch_delay_ms: 100,
        pool_size: 10,
        retry_attempts: 3,
        retry_delay_ms: 1000,
        monitoring_port: None,
        parallel_workers: 1,
        token_limit: 8000,
        pool_max_size: 10,
        pool_timeout_secs: 30,
        pool_wait_timeout_secs: 10,
        pool_create_timeout_secs: 30,
        pool_recycle_timeout_secs: 30,
    };

    // Validate config
    config.validate()?;

    // Create embedder
    let embedder = Embedder::new(Arc::new(config))?;
    info!("Created embedder with model: {}", embedder.model_name());

    // Test various texts
    let test_cases = vec![
        ("rust", "Rust programming language for systems programming"),
        ("python", "Python is a high-level interpreted language"),
        ("multilingual", "这是一个多语言测试 - This is a multilingual test"),
        ("code", "fn main() { println!(\"Hello, world!\"); }"),
        ("repo", "Repository: rust-lang/rust\nDescription: The Rust programming language\nLanguage: Rust\nStars: 90000\nOwner: rust-lang"),
    ];

    // Create validator
    let validator = together_e5_validator();
    info!("Using validator for multilingual-e5-large model (1024 dimensions)");

    for (name, text) in &test_cases {
        info!("Testing embedding for: {}", name);
        
        match embedder.generate_embedding(text).await {
            Ok(embedding) => {
                info!("Generated embedding with {} dimensions", embedding.len());
                
                // Validate the embedding
                match validator.validate(&embedding, name) {
                    Ok(_) => {
                        info!("✅ Embedding validation passed for '{}'", name);
                        
                        // Calculate some statistics
                        let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
                        let mean: f32 = embedding.iter().sum::<f32>() / embedding.len() as f32;
                        let non_zero: usize = embedding.iter().filter(|&&x| x != 0.0).count();
                        
                        info!("  Magnitude: {:.3}", magnitude);
                        info!("  Mean: {:.6}", mean);
                        info!("  Non-zero values: {} / {}", non_zero, embedding.len());
                    }
                    Err(e) => {
                        info!("❌ Embedding validation failed for '{}': {}", name, e);
                    }
                }
            }
            Err(e) => {
                info!("Failed to generate embedding for '{}': {}", name, e);
            }
        }
        
        println!(); // Add spacing between tests
    }

    // Test batch validation
    info!("Testing batch validation...");
    let mut batch_embeddings = Vec::new();
    
    for (name, text) in &test_cases[..3] {
        match embedder.generate_embedding(text).await {
            Ok(embedding) => {
                batch_embeddings.push((name.to_string(), embedding));
            }
            Err(e) => {
                info!("Failed to generate embedding for batch test: {}", e);
            }
        }
    }
    
    let batch_result = validator.validate_batch(&batch_embeddings);
    info!("Batch validation results:");
    info!("  Valid: {}", batch_result.valid);
    info!("  Invalid: {}", batch_result.invalid);
    info!("  Success rate: {:.1}%", batch_result.success_rate() * 100.0);
    
    if let Some(avg_mag) = batch_result.average_magnitude() {
        info!("  Average magnitude: {:.3}", avg_mag);
    }
    
    if !batch_result.errors.is_empty() {
        info!("  Errors:");
        for (context, error) in &batch_result.errors {
            info!("    {}: {}", context, error);
        }
    }

    Ok(())
}