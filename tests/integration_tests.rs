use embed_star::config::Config;
use embed_star::models::{ Repo, RepoOwner };
use embed_star::error::EmbedError;
use chrono::Utc;
use surrealdb::RecordId;

#[test]
fn test_repo_needs_embedding() {
    let now = Utc::now();
    let earlier = now - chrono::Duration::hours(1);

    let repo = Repo {
        id: RecordId::from(("repo", "test/repo")),
        github_id: 123,
        name: "repo".to_string(),
        full_name: "test/repo".to_string(),
        description: Some("Test repo".to_string()),
        url: "https://github.com/test/repo".to_string(),
        stars: 100,
        language: Some("Rust".to_string()),
        owner: RepoOwner {
            login: "test".to_string(),
            avatar_url: "https://github.com/test.png".to_string(),
        },
        is_private: false,
        created_at: earlier,
        updated_at: now,
        embedding: None,
        embedding_generated_at: None,
    };

    assert!(repo.needs_embedding());

    let repo_with_embedding = Repo {
        embedding: Some(vec![0.1, 0.2, 0.3]),
        embedding_generated_at: Some(earlier),
        ..repo.clone()
    };

    // Should need embedding because updated_at > embedding_generated_at
    assert!(repo_with_embedding.needs_embedding());

    let repo_up_to_date = Repo {
        updated_at: earlier,
        embedding_generated_at: Some(now),
        ..repo_with_embedding
    };

    // Should not need embedding
    assert!(!repo_up_to_date.needs_embedding());
}

#[test]
fn test_prepare_text_for_embedding() {
    let repo = Repo {
        id: RecordId::from(("repo", "rust-lang/rust")),
        github_id: 123,
        name: "rust".to_string(),
        full_name: "rust-lang/rust".to_string(),
        description: Some("The Rust programming language".to_string()),
        url: "https://github.com/rust-lang/rust".to_string(),
        stars: 90000,
        language: Some("Rust".to_string()),
        owner: RepoOwner {
            login: "rust-lang".to_string(),
            avatar_url: "https://github.com/rust-lang.png".to_string(),
        },
        is_private: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        embedding: None,
        embedding_generated_at: None,
    };

    let text = repo.prepare_text_for_embedding();
    assert!(text.contains("Repository: rust-lang/rust"));
    assert!(text.contains("Description: The Rust programming language"));
    assert!(text.contains("Language: Rust"));
    assert!(text.contains("Stars: 90000"));
    assert!(text.contains("Owner: rust-lang"));
}

#[test]
fn test_error_retryable() {
    assert!(EmbedError::ServiceUnavailable("test".to_string()).is_retryable());
    assert!((EmbedError::RateLimitExceeded { provider: "test".to_string() }).is_retryable());

    assert!(!EmbedError::Configuration("test".to_string()).is_retryable());
    assert!(!(EmbedError::InvalidDimension { expected: 100, actual: 50 }).is_retryable());
}

#[test]
fn test_config_validation() {
    let mut config = Config {
        db_url: "ws://localhost:8000".to_string(),
        db_user: "root".to_string(),
        db_pass: "root".to_string(),
        db_namespace: "test".to_string(),
        db_database: "test".to_string(),
        embedding_provider: "openai".to_string(),
        ollama_url: "http://localhost:11434".to_string(),
        openai_api_key: None,
        together_api_key: None,
        embedding_model: "text-embedding-3-small".to_string(),
        batch_size: 10,
        pool_size: 10,
        retry_attempts: 3,
        retry_delay_ms: 1000,
        batch_delay_ms: 100,
        monitoring_port: Some(9090),
        parallel_workers: 3,
        token_limit: 8000,
        pool_max_size: 10,
        pool_timeout_secs: 30,
        pool_wait_timeout_secs: 10,
        pool_create_timeout_secs: 30,
        pool_recycle_timeout_secs: 30,
    };

    // Should fail - OpenAI provider without API key
    assert!(config.validate().is_err());

    config.openai_api_key = Some("sk-test".to_string());
    assert!(config.validate().is_ok());

    // Test Together AI validation
    config.embedding_provider = "together".to_string();
    config.together_api_key = None;
    assert!(config.validate().is_err());

    config.together_api_key = Some("test-key".to_string());
    assert!(config.validate().is_ok());

    // Test batch size validation
    config.batch_size = 0;
    assert!(config.validate().is_err());
}
