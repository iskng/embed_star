use embed_star::{
    embedding_validation::EmbeddingValidator,
    metrics::Metrics,
};
use prometheus::Registry;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("embed_star=info".parse()?)
        )
        .init();

    info!("Testing embedding validation failure cases");

    // Initialize metrics
    let registry = Registry::new();
    Metrics::register(&registry)?;
    info!("Metrics initialized");

    // Create a strict validator
    let validator = EmbeddingValidator::new()
        .with_dimension(1024)
        .with_magnitude_range(0.8, 1.2); // Strict range for normalized embeddings

    // Test cases that should fail
    let test_cases = vec![
        ("empty", vec![], "Empty embedding"),
        ("wrong_dim", vec![0.1; 512], "Wrong dimension (512 instead of 1024)"),
        ("all_zeros", vec![0.0; 1024], "All zeros (no magnitude)"),
        ("has_nan", {
            let mut v = vec![0.1; 1024];
            v[10] = f32::NAN;
            v
        }, "Contains NaN"),
        ("too_sparse", {
            let mut v = vec![0.0; 1024];
            for i in 0..100 {
                v[i] = 0.1;
            }
            v
        }, "Too many zeros"),
        ("no_variance", vec![0.5; 1024], "No variance"),
    ];

    info!("\nTesting validation failures:");
    for (name, embedding, description) in &test_cases {
        match validator.validate(embedding, name) {
            Ok(_) => {
                info!("❌ UNEXPECTED PASS for '{}' ({})", name, description);
            }
            Err(e) => {
                info!("✅ Expected failure for '{}': {}", name, e);
                
                // Record the validation failure metric
                embed_star::metrics::record_embedding_validation("test_model", false);
            }
        }
    }

    // Test a valid embedding
    let valid_embedding: Vec<f32> = (0..1024)
        .map(|i| {
            let x = i as f32 / 1024.0;
            (x * std::f32::consts::PI * 4.0).sin() * 0.03
        })
        .collect();
    
    // Normalize to magnitude ~1.0
    let magnitude: f32 = valid_embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    let normalized: Vec<f32> = valid_embedding.iter().map(|x| x / magnitude).collect();
    
    match validator.validate(&normalized, "valid_normalized") {
        Ok(_) => {
            info!("✅ Valid embedding passed validation");
            embed_star::metrics::record_embedding_validation("test_model", true);
        }
        Err(e) => {
            info!("❌ Unexpected failure for valid embedding: {}", e);
        }
    }

    // Check metrics
    let metrics_text = prometheus::TextEncoder::new()
        .encode_to_string(&registry.gather())
        .unwrap();
    
    info!("\nValidation metrics:");
    for line in metrics_text.lines() {
        if line.contains("embed_star_embedding_validations_total") && !line.starts_with("#") {
            info!("  {}", line);
        }
    }

    Ok(())
}