use crate::error::{EmbedError, Result};

/// Validates embeddings to ensure they meet quality standards
pub struct EmbeddingValidator {
    expected_dimension: Option<usize>,
    min_magnitude: f32,
    max_magnitude: f32,
    max_zero_ratio: f32,
}

impl Default for EmbeddingValidator {
    fn default() -> Self {
        Self {
            expected_dimension: None,
            min_magnitude: 0.1,
            max_magnitude: 10.0,
            max_zero_ratio: 0.5,
        }
    }
}

impl EmbeddingValidator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_dimension(mut self, dimension: usize) -> Self {
        self.expected_dimension = Some(dimension);
        self
    }

    pub fn with_magnitude_range(mut self, min: f32, max: f32) -> Self {
        self.min_magnitude = min;
        self.max_magnitude = max;
        self
    }

    pub fn validate(&self, embedding: &[f32], context: &str) -> Result<()> {
        // Check if embedding is empty
        if embedding.is_empty() {
            return Err(EmbedError::InvalidEmbedding(
                format!("{}: Embedding is empty", context)
            ));
        }

        // Check dimension if specified
        if let Some(expected_dim) = self.expected_dimension {
            if embedding.len() != expected_dim {
                return Err(EmbedError::InvalidDimension {
                    expected: expected_dim,
                    actual: embedding.len(),
                });
            }
        }

        // Check for NaN or infinite values
        let invalid_count = embedding.iter().filter(|&&x| !x.is_finite()).count();
        if invalid_count > 0 {
            return Err(EmbedError::InvalidEmbedding(
                format!("{}: Contains {} non-finite values", context, invalid_count)
            ));
        }

        // Calculate magnitude
        let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        
        // Check magnitude is in reasonable range
        if magnitude < self.min_magnitude {
            return Err(EmbedError::InvalidEmbedding(
                format!(
                    "{}: Magnitude {} is below minimum {}",
                    context, magnitude, self.min_magnitude
                )
            ));
        }

        if magnitude > self.max_magnitude {
            return Err(EmbedError::InvalidEmbedding(
                format!(
                    "{}: Magnitude {} exceeds maximum {}",
                    context, magnitude, self.max_magnitude
                )
            ));
        }

        // Check for too many zeros (indicates potential issue)
        let zero_count = embedding.iter().filter(|&&x| x == 0.0).count();
        let zero_ratio = zero_count as f32 / embedding.len() as f32;
        
        if zero_ratio > self.max_zero_ratio {
            return Err(EmbedError::InvalidEmbedding(
                format!(
                    "{}: {:.1}% of values are zero (max allowed: {:.1}%)",
                    context,
                    zero_ratio * 100.0,
                    self.max_zero_ratio * 100.0
                )
            ));
        }

        // Check variance (all same value is suspicious)
        let mean: f32 = embedding.iter().sum::<f32>() / embedding.len() as f32;
        let variance: f32 = embedding
            .iter()
            .map(|x| (x - mean).powi(2))
            .sum::<f32>() / embedding.len() as f32;
        
        if variance < 1e-6 {
            return Err(EmbedError::InvalidEmbedding(
                format!("{}: Variance too low, all values nearly identical", context)
            ));
        }

        Ok(())
    }

    /// Validates a batch of embeddings and returns detailed statistics
    pub fn validate_batch(&self, embeddings: &[(String, Vec<f32>)]) -> BatchValidationResult {
        let mut results = BatchValidationResult::default();
        
        for (context, embedding) in embeddings {
            match self.validate(embedding, context) {
                Ok(_) => {
                    results.valid += 1;
                    
                    // Collect statistics
                    let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
                    results.magnitudes.push(magnitude);
                    
                    if results.dimension.is_none() {
                        results.dimension = Some(embedding.len());
                    }
                }
                Err(e) => {
                    results.invalid += 1;
                    results.errors.push((context.clone(), e.to_string()));
                }
            }
        }
        
        results
    }
}

#[derive(Debug, Default)]
pub struct BatchValidationResult {
    pub valid: usize,
    pub invalid: usize,
    pub errors: Vec<(String, String)>,
    pub magnitudes: Vec<f32>,
    pub dimension: Option<usize>,
}

impl BatchValidationResult {
    pub fn success_rate(&self) -> f32 {
        if self.valid + self.invalid == 0 {
            0.0
        } else {
            self.valid as f32 / (self.valid + self.invalid) as f32
        }
    }

    pub fn average_magnitude(&self) -> Option<f32> {
        if self.magnitudes.is_empty() {
            None
        } else {
            Some(self.magnitudes.iter().sum::<f32>() / self.magnitudes.len() as f32)
        }
    }
}

/// Specific validator for Together AI multilingual-e5-large model
pub fn together_e5_validator() -> EmbeddingValidator {
    EmbeddingValidator::new()
        .with_dimension(1024)
        .with_magnitude_range(0.5, 2.0) // Normalized embeddings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_embedding() {
        let validator = EmbeddingValidator::new();
        let embedding = vec![0.1, 0.2, -0.3, 0.4, -0.5];
        
        assert!(validator.validate(&embedding, "test").is_ok());
    }

    #[test]
    fn test_empty_embedding() {
        let validator = EmbeddingValidator::new();
        let embedding = vec![];
        
        let result = validator.validate(&embedding, "test");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_dimension_check() {
        let validator = EmbeddingValidator::new().with_dimension(5);
        
        let correct_dim = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        assert!(validator.validate(&correct_dim, "test").is_ok());
        
        let wrong_dim = vec![0.1, 0.2, 0.3];
        let result = validator.validate(&wrong_dim, "test");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            EmbedError::InvalidDimension { expected: 5, actual: 3 }
        ));
    }

    #[test]
    fn test_nan_detection() {
        let validator = EmbeddingValidator::new();
        let embedding = vec![0.1, f32::NAN, 0.3];
        
        let result = validator.validate(&embedding, "test");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("non-finite"));
    }

    #[test]
    fn test_magnitude_check() {
        let validator = EmbeddingValidator::new().with_magnitude_range(0.5, 2.0);
        
        // Too small magnitude
        let small = vec![0.01, 0.01, 0.01];
        assert!(validator.validate(&small, "test").is_err());
        
        // Good magnitude with variance
        let good = vec![0.4, 0.5, 0.6]; // magnitude ≈ 0.866 with variance
        assert!(validator.validate(&good, "test").is_ok());
        
        // Too large magnitude
        let large = vec![10.0, 10.0, 10.0];
        assert!(validator.validate(&large, "test").is_err());
    }

    #[test]
    fn test_zero_ratio() {
        let validator = EmbeddingValidator {
            max_zero_ratio: 0.5,
            ..Default::default()
        };
        
        // 40% zeros - should pass
        let some_zeros = vec![0.0, 0.0, 0.5, 0.5, 0.5];
        assert!(validator.validate(&some_zeros, "test").is_ok());
        
        // 60% zeros - should fail
        let many_zeros = vec![0.0, 0.0, 0.0, 0.5, 0.5];
        let result = validator.validate(&many_zeros, "test");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("60.0% of values are zero"));
    }

    #[test]
    fn test_batch_validation() {
        let validator = EmbeddingValidator::new().with_dimension(3);
        
        let batch = vec![
            ("repo1".to_string(), vec![0.1, 0.2, 0.3]),
            ("repo2".to_string(), vec![0.4, 0.5, 0.6]),
            ("repo3".to_string(), vec![0.0, 0.0]), // Wrong dimension
            ("repo4".to_string(), vec![f32::NAN, 0.1, 0.2]), // Contains NaN
        ];
        
        let result = validator.validate_batch(&batch);
        
        assert_eq!(result.valid, 2);
        assert_eq!(result.invalid, 2);
        assert_eq!(result.errors.len(), 2);
        assert_eq!(result.success_rate(), 0.5);
        assert!(result.average_magnitude().is_some());
    }
}