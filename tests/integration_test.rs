/// Integration tests for embed_star service
///
/// These tests verify the complete embedding pipeline works correctly
/// Note: Requires SurrealDB running locally on port 8000

use anyhow::Result;
use surrealdb::{ engine::any::{ Any, connect }, opt::auth::Root, sql::Datetime, RecordId, Surreal };
use std::time::Duration;
use tokio::time::{ sleep, timeout };

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct TestRepo {
    id: RecordId,
    full_name: String,
    description: Option<String>,
    language: Option<String>,
    stars: i64,
    owner_login: String,
    created_at: Datetime,
    updated_at: Datetime,
    embedding: Option<Vec<f32>>,
    embedding_generated_at: Option<Datetime>,
}

/// Test database connection and basic operations
#[tokio::test]
async fn test_database_connection() -> Result<()> {
    let db = create_test_db().await?;

    // Test basic query
    let mut response = db.query("RETURN 'hello'").await?;
    let result: Option<String> = response.take(0)?;

    assert_eq!(result, Some("hello".to_string()));

    cleanup_test_db(&db).await?;
    Ok(())
}

/// Test repository creation and retrieval
#[tokio::test]
async fn test_repo_operations() -> Result<()> {
    let db = create_test_db().await?;
    setup_schema(&db).await?;

    // Create a test repository
    let repo_data = TestRepo {
        id: RecordId::from(("repo".to_string(), "test123".to_string())),
        full_name: "test/repo".to_string(),
        description: Some("Test repository".to_string()),
        language: Some("Rust".to_string()),
        stars: 100,
        owner_login: "test".to_string(),
        created_at: Datetime::from(chrono::Utc::now()),
        updated_at: Datetime::from(chrono::Utc::now()),
        embedding: None,
        embedding_generated_at: None,
    };

    // Insert repo
    let created: Option<TestRepo> = db.create("repo").content(repo_data).await?;

    assert!(created.is_some());
    let created_repo = created.unwrap();
    assert_eq!(created_repo.full_name, "test/repo");
    assert!(created_repo.embedding.is_none());

    // Verify repo was created
    let query = "SELECT * FROM repo WHERE id = $id";
    let mut response = db.query(query).bind(("id", created_repo.id.clone())).await?;
    let fetched_repos: Vec<TestRepo> = response.take(0)?;

    assert_eq!(fetched_repos.len(), 1);
    assert_eq!(fetched_repos[0].full_name, "test/repo");
    assert!(fetched_repos[0].embedding.is_none());

    cleanup_test_db(&db).await?;
    Ok(())
}

/// Test embedding generation (requires Ollama running)
#[tokio::test]
#[ignore = "Requires Ollama running locally"]
async fn test_embedding_generation() -> Result<()> {
    use embed_star::embedder::Embedder;
    use embed_star::config::Config;
    use std::sync::Arc;

    // Create config for Ollama
    let config = Config {
        db_url: "ws://localhost:8000".to_string(),
        db_user: "root".to_string(),
        db_pass: "root".to_string(),
        db_namespace: "test".to_string(),
        db_database: "embed_star_test".to_string(),
        embedding_provider: "ollama".to_string(),
        ollama_url: "http://localhost:11434".to_string(),
        openai_api_key: None,
        together_api_key: None,
        batch_size: 10,
        batch_delay_ms: 100,
        pool_size: 10,
        retry_attempts: 3,
        retry_delay_ms: 1000,
        monitoring_port: Some(9090),
        parallel_workers: 1,
        token_limit: 8000,
        pool_max_size: 10,
        pool_timeout_secs: 30,
        pool_wait_timeout_secs: 10,
        pool_create_timeout_secs: 30,
        pool_recycle_timeout_secs: 30,
        embedding_model: "nomic-embed-text".to_string(),
    };

    let embedder = Embedder::new(Arc::new(config))?;

    // Test embedding generation
    let text = "The Rust programming language";
    let embedding = embedder.generate_embedding(text).await?;

    // Verify embedding properties
    assert!(!embedding.is_empty());
    assert!(embedding.len() > 100); // Most models generate 100+ dimensions

    // Check values are reasonable
    let magnitude: f32 = embedding
        .iter()
        .map(|x| x * x)
        .sum::<f32>()
        .sqrt();
    assert!(magnitude > 0.1, "Embedding magnitude too small");
    assert!(magnitude < 100.0, "Embedding magnitude too large");

    Ok(())
}

/// Test the complete embedding pipeline
#[tokio::test]
#[ignore = "Requires SurrealDB and Ollama running"]
async fn test_full_pipeline() -> Result<()> {
    // Setup test environment
    std::env::set_var("DB_URL", "ws://localhost:8000");
    std::env::set_var("DB_NAMESPACE", "test");
    std::env::set_var("DB_DATABASE", "embed_star_test");
    std::env::set_var("EMBEDDING_PROVIDER", "ollama");
    std::env::set_var("BATCH_SIZE", "2");
    std::env::set_var("PARALLEL_WORKERS", "1");

    let db = create_test_db().await?;
    setup_schema(&db).await?;

    // Create test repositories
    for i in 0..5 {
        let created: Option<TestRepo> = db.create("repo").content(TestRepo {
            id: RecordId::from(("repo".to_string(), format!("test{}", i))),
            full_name: format!("test/repo{}", i),
            description: Some(format!("Test repository {}", i)),
            language: Some("Rust".to_string()),
            stars: 100 + i,
            owner_login: "test".to_string(),
            created_at: Datetime::from(chrono::Utc::now()),
            updated_at: Datetime::from(chrono::Utc::now()),
            embedding: None,
            embedding_generated_at: None,
        }).await?;
        assert!(created.is_some());
    }

    // Start the service in a background task
    let service_handle = tokio::spawn(async {
        if let Err(e) = embed_star::run_service().await {
            eprintln!("Service error: {}", e);
        }
    });

    // Wait for embeddings to be generated (with timeout)
    let check_result = timeout(Duration::from_secs(30), async {
        loop {
            let mut response = db.query(
                "SELECT count() FROM repo WHERE embedding IS NOT NULL GROUP ALL"
            ).await?;
            let count_val: Option<serde_json::Value> = response.take(0)?;

            let count = match count_val {
                Some(val) => {
                    if let Some(c) = val.get("count").and_then(|v| v.as_i64()) {
                        c as usize
                    } else {
                        0
                    }
                }
                None => 0,
            };

            if count >= 5 {
                break;
            }

            sleep(Duration::from_secs(1)).await;
        }
        Ok::<(), anyhow::Error>(())
    }).await;

    // Stop the service
    service_handle.abort();

    // Verify results
    check_result??;

    let mut response = db.query("SELECT * FROM repo").await?;
    let repos: Vec<TestRepo> = response.take(0)?;
    assert_eq!(repos.len(), 5);

    for repo in repos {
        assert!(repo.embedding.is_some(), "Repo {} missing embedding", repo.full_name);
        assert!(repo.embedding_generated_at.is_some());

        let embedding = repo.embedding.unwrap();
        assert!(!embedding.is_empty());
    }

    cleanup_test_db(&db).await?;
    Ok(())
}

/// Helper function to create test database connection
async fn create_test_db() -> Result<Surreal<Any>> {
    let db: Surreal<Any> = connect("ws://localhost:8000").await?;

    db.signin(Root {
        username: "root",
        password: "root",
    }).await?;

    db.use_ns("test").use_db("embed_star_test").await?;

    Ok(db)
}

/// Helper function to set up database schema
async fn setup_schema(db: &Surreal<Any>) -> Result<()> {
    // Clean existing data
    let _ = db.query("DELETE repo").await?;

    // For testing, use schemaless mode
    let _ = db.query(
        r#"
        DEFINE TABLE repo SCHEMALESS;
        
        DEFINE INDEX idx_repo_full_name ON TABLE repo COLUMNS full_name;
    "#
    ).await?;

    Ok(())
}

/// Helper function to clean up test database
async fn cleanup_test_db(db: &Surreal<Any>) -> Result<()> {
    let _ = db.query("DELETE repo").await?;
    Ok(())
}
