use embed_star::embedding_validation::{EmbeddingValidator, BatchValidationResult};
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

    info!("Testing embedding validation edge cases");

    // Create validators with different configurations
    let strict_validator = EmbeddingValidator::new()
        .with_dimension(1024)
        .with_magnitude_range(0.5, 2.0);

    let lenient_validator = EmbeddingValidator::new()
        .with_dimension(1024)
        .with_magnitude_range(0.1, 10.0);

    // Test cases with artificial embeddings
    let test_cases: Vec<(&str, Vec<f32>, &str)> = vec![
        (
            "normal_embedding",
            create_normal_embedding(1024),
            "Normal embedding with proper distribution"
        ),
        (
            "zero_embedding",
            vec![0.0; 1024],
            "All zeros (should fail)"
        ),
        (
            "constant_embedding",
            vec![0.5; 1024],
            "All same value (should fail due to no variance)"
        ),
        (
            "tiny_magnitude",
            create_scaled_embedding(1024, 0.01),
            "Very small magnitude"
        ),
        (
            "large_magnitude",
            create_scaled_embedding(1024, 10.0),
            "Very large magnitude"
        ),
        (
            "wrong_dimension",
            create_normal_embedding(512),
            "Wrong dimension (512 instead of 1024)"
        ),
        (
            "contains_nan",
            create_embedding_with_nan(1024),
            "Contains NaN values"
        ),
        (
            "many_zeros",
            create_sparse_embedding(1024, 0.8),
            "80% zeros (sparse embedding)"
        ),
    ];

    println!("\n=== Testing with STRICT validator ===");
    test_embeddings(&strict_validator, &test_cases);

    println!("\n=== Testing with LENIENT validator ===");
    test_embeddings(&lenient_validator, &test_cases);

    // Test batch validation
    println!("\n=== Batch Validation Test ===");
    let batch: Vec<(String, Vec<f32>)> = test_cases.iter()
        .take(5)
        .map(|(name, emb, _)| (name.to_string(), emb.clone()))
        .collect();

    let batch_result = strict_validator.validate_batch(&batch);
    print_batch_results(&batch_result);

    Ok(())
}

fn test_embeddings(validator: &EmbeddingValidator, test_cases: &[(&str, Vec<f32>, &str)]) {
    for (name, embedding, description) in test_cases {
        print!("{:<20} ({:<40}): ", name, description);
        match validator.validate(embedding, name) {
            Ok(_) => println!("✅ PASS"),
            Err(e) => println!("❌ FAIL - {}", e),
        }
    }
}

fn print_batch_results(results: &BatchValidationResult) {
    println!("Batch validation results:");
    println!("  Total: {}", results.valid + results.invalid);
    println!("  Valid: {}", results.valid);
    println!("  Invalid: {}", results.invalid);
    println!("  Success rate: {:.1}%", results.success_rate() * 100.0);
    
    if let Some(avg_mag) = results.average_magnitude() {
        println!("  Average magnitude: {:.3}", avg_mag);
    }
    
    if !results.errors.is_empty() {
        println!("\nErrors:");
        for (context, error) in &results.errors {
            println!("  - {}: {}", context, error);
        }
    }
}

// Helper functions to create test embeddings

fn create_normal_embedding(dim: usize) -> Vec<f32> {
    use std::f32::consts::PI;
    (0..dim)
        .map(|i| {
            let x = i as f32 / dim as f32;
            (2.0 * PI * x * 3.0).sin() * 0.5 + (PI * x * 7.0).cos() * 0.3
        })
        .collect()
}

fn create_scaled_embedding(dim: usize, scale: f32) -> Vec<f32> {
    create_normal_embedding(dim)
        .into_iter()
        .map(|x| x * scale)
        .collect()
}

fn create_embedding_with_nan(dim: usize) -> Vec<f32> {
    let mut emb = create_normal_embedding(dim);
    emb[10] = f32::NAN;
    emb[20] = f32::INFINITY;
    emb
}

fn create_sparse_embedding(dim: usize, zero_ratio: f32) -> Vec<f32> {
    create_normal_embedding(dim)
        .into_iter()
        .enumerate()
        .map(|(i, x)| {
            if (i as f32 / dim as f32) < zero_ratio {
                0.0
            } else {
                x
            }
        })
        .collect()
}