use embed_star::{
    config::Config,
    embedder::{Embedder, EmbeddingProvider, TogetherAIEmbedder},
};
use std::sync::Arc;

#[tokio::test]
#[ignore] // Run with: cargo test test_together_api -- --ignored
async fn test_together_api_connection() {
    // This test requires a valid TOGETHER_API_KEY environment variable
    let api_key = match std::env::var("TOGETHER_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("Skipping test: TOGETHER_API_KEY not set");
            return;
        }
    };

    let embedder = TogetherAIEmbedder::new(
        &api_key,
        "togethercomputer/m2-bert-80M-8k-retrieval".to_string(),
    )
    .expect("Failed to create Together AI embedder");

    // Test with a simple text
    let text = "This is a test of the Together AI embedding API";
    let result = embedder.generate_embedding(text).await;

    assert!(result.is_ok(), "Failed to generate embedding: {:?}", result);
    
    let embedding = result.unwrap();
    assert!(!embedding.is_empty(), "Embedding should not be empty");
    assert!(embedding.len() > 100, "Embedding dimensions should be > 100");
    
    // Check that all values are finite
    for value in &embedding {
        assert!(value.is_finite(), "Embedding contains non-finite values");
    }
    
    println!("Successfully generated embedding with {} dimensions", embedding.len());
}

#[tokio::test]
#[ignore] // Run with: cargo test test_together_multilingual -- --ignored
async fn test_together_multilingual_e5() {
    // Test the specific model used in production
    let api_key = match std::env::var("TOGETHER_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("Skipping test: TOGETHER_API_KEY not set");
            return;
        }
    };

    let embedder = TogetherAIEmbedder::new(
        &api_key,
        "intfloat/multilingual-e5-large-instruct".to_string(),
    )
    .expect("Failed to create Together AI embedder");

    // Test with multiple languages
    let test_texts = vec![
        ("English", "The Rust programming language"),
        ("Spanish", "El lenguaje de programación Rust"),
        ("Japanese", "Rustプログラミング言語"),
        ("Code", "fn main() { println!(\"Hello, world!\"); }"),
    ];

    for (lang, text) in test_texts {
        let result = embedder.generate_embedding(text).await;
        assert!(
            result.is_ok(),
            "Failed to generate {} embedding: {:?}",
            lang,
            result
        );
        
        let embedding = result.unwrap();
        assert_eq!(
            embedding.len(),
            1024,
            "{} embedding should have 1024 dimensions",
            lang
        );
        
        // Check magnitude is reasonable (normalized embeddings)
        let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            magnitude > 0.1 && magnitude < 10.0,
            "{} embedding magnitude {} is out of expected range",
            lang,
            magnitude
        );
        
        println!("{} embedding: OK (magnitude: {:.3})", lang, magnitude);
    }
}

#[tokio::test]
#[ignore] // Run with: cargo test test_together_similarity -- --ignored
async fn test_together_similarity() {
    // Test that similar texts produce similar embeddings
    let api_key = match std::env::var("TOGETHER_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("Skipping test: TOGETHER_API_KEY not set");
            return;
        }
    };

    let embedder = TogetherAIEmbedder::new(
        &api_key,
        "intfloat/multilingual-e5-large-instruct".to_string(),
    )
    .expect("Failed to create Together AI embedder");

    // Similar programming language descriptions
    let rust_text = "Rust is a systems programming language focused on safety and concurrency";
    let go_text = "Go is a systems programming language designed for simplicity and concurrency";
    let python_text = "Python is a high-level interpreted language for general-purpose programming";
    
    let rust_emb = embedder.generate_embedding(rust_text).await.unwrap();
    let go_emb = embedder.generate_embedding(go_text).await.unwrap();
    let python_emb = embedder.generate_embedding(python_text).await.unwrap();
    
    // Calculate cosine similarities
    let rust_go_sim = cosine_similarity(&rust_emb, &go_emb);
    let rust_python_sim = cosine_similarity(&rust_emb, &python_emb);
    let go_python_sim = cosine_similarity(&go_emb, &python_emb);
    
    println!("Similarities:");
    println!("  Rust-Go: {:.3}", rust_go_sim);
    println!("  Rust-Python: {:.3}", rust_python_sim);
    println!("  Go-Python: {:.3}", go_python_sim);
    
    // Rust and Go should be more similar than Rust and Python
    assert!(
        rust_go_sim > rust_python_sim,
        "Rust-Go similarity ({:.3}) should be higher than Rust-Python ({:.3})",
        rust_go_sim,
        rust_python_sim
    );
}

#[tokio::test]
#[ignore] // Run with: cargo test test_together_error_handling -- --ignored
async fn test_together_error_handling() {
    // Test with invalid API key
    let embedder = TogetherAIEmbedder::new(
        "invalid-api-key",
        "intfloat/multilingual-e5-large-instruct".to_string(),
    )
    .expect("Failed to create Together AI embedder");

    let result = embedder.generate_embedding("test").await;
    assert!(result.is_err(), "Should fail with invalid API key");
    
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("401") || error_msg.contains("unauthorized"),
        "Error should indicate authentication failure: {}",
        error_msg
    );
}

#[tokio::test]
#[ignore] // Run with: cargo test test_embedder_integration -- --ignored
async fn test_embedder_integration() {
    // Test the full Embedder wrapper
    let api_key = match std::env::var("TOGETHER_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("Skipping test: TOGETHER_API_KEY not set");
            return;
        }
    };

    let config = Config {
        db_url: "ws://localhost:8000".to_string(),
        db_user: "root".to_string(),
        db_pass: "root".to_string(),
        db_namespace: "test".to_string(),
        db_database: "test".to_string(),
        embedding_provider: "together".to_string(),
        ollama_url: "http://localhost:11434".to_string(),
        openai_api_key: None,
        together_api_key: Some(api_key),
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

    let embedder = Embedder::new(Arc::new(config)).expect("Failed to create embedder");
    
    // Test normal text
    let text = "Repository: rust-lang/rust\nDescription: The Rust programming language\nLanguage: Rust\nStars: 90000";
    let result = embedder.generate_embedding(text).await;
    
    assert!(result.is_ok(), "Failed to generate embedding: {:?}", result);
    let embedding = result.unwrap();
    assert_eq!(embedding.len(), 1024, "Expected 1024 dimensions");
    
    // Test text truncation
    let long_text = "a".repeat(10000); // Exceeds token limit
    let result = embedder.generate_embedding(&long_text).await;
    assert!(result.is_ok(), "Should handle long text gracefully");
}

#[tokio::test]
#[ignore] // Run with: cargo test test_together_rate_limiting -- --ignored
async fn test_together_rate_limiting() {
    // Test rapid requests to check rate limiting behavior
    let api_key = match std::env::var("TOGETHER_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("Skipping test: TOGETHER_API_KEY not set");
            return;
        }
    };

    let embedder = TogetherAIEmbedder::new(
        &api_key,
        "intfloat/multilingual-e5-large-instruct".to_string(),
    )
    .expect("Failed to create Together AI embedder");

    // Make 5 rapid requests
    let start = std::time::Instant::now();
    let mut results = Vec::new();
    
    for i in 0..5 {
        let text = format!("Test request number {}", i);
        let result = embedder.generate_embedding(&text).await;
        results.push(result);
    }
    
    let duration = start.elapsed();
    println!("5 requests completed in {:?}", duration);
    
    // All requests should succeed (Together AI has generous rate limits)
    for (i, result) in results.iter().enumerate() {
        assert!(
            result.is_ok(),
            "Request {} failed: {:?}",
            i,
            result
        );
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }

    let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let magnitude_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let magnitude_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if magnitude_a == 0.0 || magnitude_b == 0.0 {
        0.0
    } else {
        dot_product / (magnitude_a * magnitude_b)
    }
}