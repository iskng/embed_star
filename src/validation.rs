use crate::error::{EmbedError, Result};
use tracing::{debug, warn};

/// Configuration for embedding validation
#[derive(Debug, Clone)]
pub struct ValidationConfig {
    /// Minimum acceptable embedding dimension
    pub min_dimension: usize,
    /// Maximum acceptable embedding dimension
    pub max_dimension: usize,
    /// Maximum allowed zero values (as percentage)
    pub max_zero_ratio: f32,
    /// Minimum magnitude (L2 norm) for embeddings
    pub min_magnitude: f32,
    /// Maximum magnitude (L2 norm) for embeddings
    pub max_magnitude: f32,
    /// Check for NaN or infinite values
    pub check_finite: bool,
    /// Maximum allowed duplicate values (as percentage)
    pub max_duplicate_ratio: f32,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            min_dimension: 100,
            max_dimension: 4096,
            max_zero_ratio: 0.9, // Allow up to 90% zeros
            min_magnitude: 0.01,
            max_magnitude: 100.0,
            check_finite: true,
            max_duplicate_ratio: 0.5, // Allow up to 50% duplicate values
        }
    }
}

/// Validates embeddings based on various quality criteria
pub struct EmbeddingValidator {
    config: ValidationConfig,
}

impl EmbeddingValidator {
    pub fn new(config: ValidationConfig) -> Self {
        Self { config }
    }

    /// Validate an embedding vector
    pub fn validate(&self, embedding: &[f32], source: &str) -> Result<()> {
        // Check dimension
        if embedding.len() < self.config.min_dimension {
            return Err(EmbedError::ValidationError(format!(
                "Embedding dimension {} is below minimum {} for {}",
                embedding.len(),
                self.config.min_dimension,
                source
            )));
        }

        if embedding.len() > self.config.max_dimension {
            return Err(EmbedError::ValidationError(format!(
                "Embedding dimension {} exceeds maximum {} for {}",
                embedding.len(),
                self.config.max_dimension,
                source
            )));
        }

        // Check for finite values (NaN and Inf)
        if self.config.check_finite {
            for (i, &value) in embedding.iter().enumerate() {
                if value.is_nan() {
                    return Err(EmbedError::ValidationError(format!(
                        "NaN value at index {} for {}",
                        i, source
                    )));
                }
                if value.is_infinite() {
                    return Err(EmbedError::ValidationError(format!(
                        "Infinite value ({}) at index {} for {}",
                        if value.is_sign_positive() { "+Inf" } else { "-Inf" },
                        i, 
                        source
                    )));
                }
            }
        }

        // Calculate statistics
        let stats = self.calculate_stats(embedding);

        // Check zero ratio
        if stats.zero_ratio > self.config.max_zero_ratio {
            warn!(
                "High zero ratio {:.2}% for {} (threshold: {:.2}%)",
                stats.zero_ratio * 100.0,
                source,
                self.config.max_zero_ratio * 100.0
            );
            return Err(EmbedError::ValidationError(format!(
                "Zero ratio {:.2}% exceeds maximum {:.2}% for {}",
                stats.zero_ratio * 100.0,
                self.config.max_zero_ratio * 100.0,
                source
            )));
        }

        // Check magnitude
        if stats.magnitude < self.config.min_magnitude {
            return Err(EmbedError::ValidationError(format!(
                "Embedding magnitude {:.4} is below minimum {:.4} for {}",
                stats.magnitude, self.config.min_magnitude, source
            )));
        }

        if stats.magnitude > self.config.max_magnitude {
            return Err(EmbedError::ValidationError(format!(
                "Embedding magnitude {:.4} exceeds maximum {:.4} for {}",
                stats.magnitude, self.config.max_magnitude, source
            )));
        }

        // Check duplicate ratio
        if stats.duplicate_ratio > self.config.max_duplicate_ratio {
            warn!(
                "High duplicate ratio {:.2}% for {} (threshold: {:.2}%)",
                stats.duplicate_ratio * 100.0,
                source,
                self.config.max_duplicate_ratio * 100.0
            );
        }

        debug!(
            "Embedding validation passed for {}: dim={}, magnitude={:.4}, zeros={:.2}%, duplicates={:.2}%",
            source,
            embedding.len(),
            stats.magnitude,
            stats.zero_ratio * 100.0,
            stats.duplicate_ratio * 100.0
        );

        Ok(())
    }

    /// Calculate statistics for an embedding
    fn calculate_stats(&self, embedding: &[f32]) -> EmbeddingStats {
        let mut zero_count = 0;
        let mut magnitude_squared = 0.0;
        let mut value_counts = std::collections::HashMap::new();

        for &value in embedding {
            if value == 0.0 {
                zero_count += 1;
            }
            magnitude_squared += value * value;
            *value_counts.entry(value.to_bits()).or_insert(0) += 1;
        }

        let total = embedding.len() as f32;
        let zero_ratio = zero_count as f32 / total;
        let magnitude = magnitude_squared.sqrt();

        // Calculate duplicate ratio
        let max_count = value_counts.values().max().copied().unwrap_or(0);
        let duplicate_ratio = if max_count > 1 {
            (max_count - 1) as f32 / total
        } else {
            0.0
        };

        EmbeddingStats {
            zero_ratio,
            magnitude,
            duplicate_ratio,
        }
    }

    /// Normalize an embedding to unit length
    pub fn normalize(&self, embedding: &mut [f32]) -> Result<()> {
        let magnitude = embedding.iter().map(|&x| x * x).sum::<f32>().sqrt();
        
        if magnitude < self.config.min_magnitude {
            return Err(EmbedError::ValidationError(format!(
                "Cannot normalize embedding with magnitude {:.4}",
                magnitude
            )));
        }

        for value in embedding.iter_mut() {
            *value /= magnitude;
        }

        Ok(())
    }

    /// Compare two embeddings for similarity
    pub fn cosine_similarity(&self, a: &[f32], b: &[f32]) -> Result<f32> {
        if a.len() != b.len() {
            return Err(EmbedError::ValidationError(format!(
                "Dimension mismatch: {} vs {}",
                a.len(),
                b.len()
            )));
        }

        let dot_product: f32 = a.iter().zip(b.iter()).map(|(&x, &y)| x * y).sum();
        let magnitude_a: f32 = a.iter().map(|&x| x * x).sum::<f32>().sqrt();
        let magnitude_b: f32 = b.iter().map(|&x| x * x).sum::<f32>().sqrt();

        if magnitude_a < self.config.min_magnitude || magnitude_b < self.config.min_magnitude {
            return Err(EmbedError::ValidationError(
                "Embeddings have insufficient magnitude for similarity calculation".to_string()
            ));
        }

        Ok(dot_product / (magnitude_a * magnitude_b))
    }
}

#[derive(Debug)]
struct EmbeddingStats {
    zero_ratio: f32,
    magnitude: f32,
    duplicate_ratio: f32,
}

/// Quality metrics for embedding providers
pub struct ProviderQualityMetrics {
    pub provider: String,
    pub total_validations: u64,
    pub failed_validations: u64,
    pub average_magnitude: f32,
    pub average_zero_ratio: f32,
}

impl ProviderQualityMetrics {
    pub fn new(provider: String) -> Self {
        Self {
            provider,
            total_validations: 0,
            failed_validations: 0,
            average_magnitude: 0.0,
            average_zero_ratio: 0.0,
        }
    }

    pub fn update(&mut self, passed: bool, magnitude: f32, zero_ratio: f32) {
        self.total_validations += 1;
        if !passed {
            self.failed_validations += 1;
        }
        
        // Update running averages
        let n = self.total_validations as f32;
        self.average_magnitude = (self.average_magnitude * (n - 1.0) + magnitude) / n;
        self.average_zero_ratio = (self.average_zero_ratio * (n - 1.0) + zero_ratio) / n;
    }

    pub fn failure_rate(&self) -> f32 {
        if self.total_validations == 0 {
            0.0
        } else {
            self.failed_validations as f32 / self.total_validations as f32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_embedding() {
        let validator = EmbeddingValidator::new(ValidationConfig::default());
        let mut embedding = Vec::new();
        for i in 0..200 {
            embedding.push(0.1 + (i % 5) as f32 * 0.1);
        }
        assert!(validator.validate(&embedding, "test").is_ok());
    }

    #[test]
    fn test_dimension_validation() {
        let validator = EmbeddingValidator::new(ValidationConfig {
            min_dimension: 100,
            max_dimension: 1000,
            ..Default::default()
        });

        let small = vec![0.1; 50];
        assert!(validator.validate(&small, "test").is_err());

        let large = vec![0.1; 2000];
        assert!(validator.validate(&large, "test").is_err());

        let valid = vec![0.1; 500];
        assert!(validator.validate(&valid, "test").is_ok());
    }

    #[test]
    fn test_zero_ratio_validation() {
        let validator = EmbeddingValidator::new(ValidationConfig {
            max_zero_ratio: 0.5,
            ..Default::default()
        });

        let mut embedding = vec![0.1; 200];
        // Set 60% to zero (exceeds 50% threshold)
        for i in 0..120 {
            embedding[i] = 0.0;
        }
        
        assert!(validator.validate(&embedding, "test").is_err());
    }

    #[test]
    fn test_magnitude_validation() {
        let validator = EmbeddingValidator::new(ValidationConfig {
            min_magnitude: 1.0,
            max_magnitude: 10.0,
            ..Default::default()
        });

        let small = vec![0.001; 200];
        assert!(validator.validate(&small, "test").is_err());

        let large = vec![10.0; 200];
        assert!(validator.validate(&large, "test").is_err());

        let valid = vec![0.1; 200];
        assert!(validator.validate(&valid, "test").is_ok());
    }

    #[test]
    fn test_normalize() {
        let validator = EmbeddingValidator::new(ValidationConfig::default());
        let mut embedding = vec![3.0, 4.0];
        
        validator.normalize(&mut embedding).unwrap();
        
        let magnitude = embedding.iter().map(|&x| x * x).sum::<f32>().sqrt();
        assert!((magnitude - 1.0).abs() < 0.0001);
        assert!((embedding[0] - 0.6).abs() < 0.0001);
        assert!((embedding[1] - 0.8).abs() < 0.0001);
    }

    #[test]
    fn test_cosine_similarity() {
        let validator = EmbeddingValidator::new(ValidationConfig::default());
        
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let c = vec![0.0, 1.0, 0.0];
        
        // Same vectors should have similarity 1.0
        let sim1 = validator.cosine_similarity(&a, &b).unwrap();
        assert!((sim1 - 1.0).abs() < 0.0001);
        
        // Orthogonal vectors should have similarity 0.0
        let sim2 = validator.cosine_similarity(&a, &c).unwrap();
        assert!(sim2.abs() < 0.0001);
    }

    #[test]
    fn test_nan_validation() {
        let validator = EmbeddingValidator::new(ValidationConfig {
            check_finite: true,
            ..Default::default()
        });

        // Test NaN detection
        let mut embedding = vec![0.1; 100];
        embedding[50] = f32::NAN;
        
        let result = validator.validate(&embedding, "test");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("NaN value at index 50"));
    }

    #[test]
    fn test_infinity_validation() {
        let validator = EmbeddingValidator::new(ValidationConfig {
            check_finite: true,
            ..Default::default()
        });

        // Test positive infinity
        let mut embedding = vec![0.1; 100];
        embedding[25] = f32::INFINITY;
        
        let result = validator.validate(&embedding, "test");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("+Inf"));
        assert!(err.to_string().contains("at index 25"));

        // Test negative infinity
        let mut embedding2 = vec![0.1; 100];
        embedding2[75] = f32::NEG_INFINITY;
        
        let result2 = validator.validate(&embedding2, "test");
        assert!(result2.is_err());
        let err2 = result2.unwrap_err();
        assert!(err2.to_string().contains("-Inf"));
        assert!(err2.to_string().contains("at index 75"));
    }

    #[test]
    fn test_finite_check_disabled() {
        let validator = EmbeddingValidator::new(ValidationConfig {
            check_finite: false,
            ..Default::default()
        });

        // With check_finite disabled, NaN and Inf should pass
        let mut embedding = vec![0.1; 100];
        embedding[10] = f32::NAN;
        embedding[20] = f32::INFINITY;
        embedding[30] = f32::NEG_INFINITY;
        
        // This should not error because check_finite is false
        assert!(validator.validate(&embedding, "test").is_ok());
    }

    #[test]
    fn test_mixed_invalid_values() {
        let validator = EmbeddingValidator::new(ValidationConfig {
            check_finite: true,
            ..Default::default()
        });

        // Test that validation stops at first invalid value
        let mut embedding = vec![0.1; 100];
        embedding[10] = f32::NAN;
        embedding[20] = f32::INFINITY; // This won't be checked because validation stops at NaN
        
        let result = validator.validate(&embedding, "test");
        assert!(result.is_err());
        let err = result.unwrap_err();
        // Should report the first error (NaN at index 10)
        assert!(err.to_string().contains("NaN value at index 10"));
    }
}