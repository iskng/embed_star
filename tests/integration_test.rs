/// Integration tests for embed_star service
/// 
/// These tests verify the complete embedding pipeline works correctly
/// Note: Requires SurrealDB running locally on port 8000

use anyhow::Result;
use surrealdb::{
    engine::remote::ws::{Client, Ws},
    opt::auth::Root,
    sql::Thing,
    Surreal,
};
use std::time::Duration;
use tokio::time::{sleep, timeout};

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct TestRepo {
    id: Thing,
    full_name: String,
    description: Option<String>,
    language: Option<String>,
    stars: i64,
    owner_login: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    embedding: Option<Vec<f32>>,
    embedding_model: Option<String>,
    embedding_generated_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Test database connection and basic operations
#[tokio::test]
async fn test_database_connection() -> Result<()> {
    let db = create_test_db().await?;
    
    // Test basic query
    let result: Option<String> = db
        .query("RETURN 'hello'")
        .await?
        .take(0)?;
    
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
        id: Thing::from(("repo".to_string(), "test123".to_string())),
        full_name: "test/repo".to_string(),
        description: Some("Test repository".to_string()),
        language: Some("Rust".to_string()),
        stars: 100,
        owner_login: "test".to_string(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        embedding: None,
        embedding_model: None,
        embedding_generated_at: None,
    };
    
    // Insert repo
    let created: Vec<TestRepo> = db
        .create("repo")
        .content(repo_data)
        .await?;
    
    assert!(!created.is_empty());
    let created_repo = &created[0];
    assert_eq!(created_repo.full_name, "test/repo");
    assert!(created_repo.embedding.is_none());
    
    // Query repos needing embeddings
    let query = r#"
        SELECT * FROM repo
        WHERE embedding IS NULL
        LIMIT 10
    "#;
    
    let repos: Vec<TestRepo> = db.query(query).await?.take(0)?;
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].full_name, "test/repo");
    
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
        embedding_model: "nomic-embed-text".to_string(),
        batch_size: 10,
        batch_delay_ms: 100,
        pool_size: 10,
        retry_attempts: 3,
        retry_delay_ms: 1000,
        monitoring_port: Some(9090),
        parallel_workers: 1,
    };
    
    let embedder = Embedder::new(Arc::new(config))?;
    
    // Test embedding generation
    let text = "The Rust programming language";
    let embedding = embedder.generate_embedding(text).await?;
    
    // Verify embedding properties
    assert!(!embedding.is_empty());
    assert!(embedding.len() > 100); // Most models generate 100+ dimensions
    
    // Check values are reasonable
    let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
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
        let repos: Vec<TestRepo> = db
            .create("repo")
            .content(TestRepo {
                id: Thing::from(("repo".to_string(), format!("test{}", i))),
                full_name: format!("test/repo{}", i),
                description: Some(format!("Test repository {}", i)),
                language: Some("Rust".to_string()),
                stars: 100 + i,
                owner_login: "test".to_string(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                embedding: None,
                embedding_model: None,
                embedding_generated_at: None,
            })
            .await?;
        assert!(!repos.is_empty());
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
            let count: Option<usize> = db
                .query("SELECT count() FROM repo WHERE embedding IS NOT NULL GROUP ALL")
                .await?
                .take(0)?;
            
            if count.unwrap_or(0) >= 5 {
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
    
    let repos: Vec<TestRepo> = db.query("SELECT * FROM repo").await?.take(0)?;
    assert_eq!(repos.len(), 5);
    
    for repo in repos {
        assert!(repo.embedding.is_some(), "Repo {} missing embedding", repo.full_name);
        assert!(repo.embedding_model.is_some());
        assert!(repo.embedding_generated_at.is_some());
        
        let embedding = repo.embedding.unwrap();
        assert!(!embedding.is_empty());
    }
    
    cleanup_test_db(&db).await?;
    Ok(())
}

/// Helper function to create test database connection
async fn create_test_db() -> Result<Surreal<Client>> {
    let db = Surreal::new::<Ws>("ws://localhost:8000").await?;
    
    db.signin(Root {
        username: "root",
        password: "root",
    })
    .await?;
    
    db.use_ns("test").use_db("embed_star_test").await?;
    
    Ok(db)
}

/// Helper function to set up database schema
async fn setup_schema(db: &Surreal<Client>) -> Result<()> {
    // Clean existing data
    db.query("DELETE repo").await?;
    db.query("DELETE processing_lock").await?;
    
    // Create schema
    db.query(r#"
        DEFINE TABLE repo SCHEMAFULL;
        DEFINE FIELD full_name ON TABLE repo TYPE string;
        DEFINE FIELD description ON TABLE repo TYPE option<string>;
        DEFINE FIELD language ON TABLE repo TYPE option<string>;
        DEFINE FIELD stars ON TABLE repo TYPE int;
        DEFINE FIELD owner_login ON TABLE repo TYPE string;
        DEFINE FIELD created_at ON TABLE repo TYPE datetime;
        DEFINE FIELD updated_at ON TABLE repo TYPE datetime;
        DEFINE FIELD embedding ON TABLE repo TYPE option<array<float>>;
        DEFINE FIELD embedding_model ON TABLE repo TYPE option<string>;
        DEFINE FIELD embedding_generated_at ON TABLE repo TYPE option<datetime>;
        
        DEFINE INDEX idx_repo_full_name ON TABLE repo COLUMNS full_name UNIQUE;
        
        DEFINE TABLE processing_lock SCHEMAFULL;
        DEFINE FIELD repo_id ON TABLE processing_lock TYPE record<repo>;
        DEFINE FIELD instance_id ON TABLE processing_lock TYPE string;
        DEFINE FIELD locked_at ON TABLE processing_lock TYPE datetime;
        DEFINE FIELD expires_at ON TABLE processing_lock TYPE datetime;
        DEFINE FIELD processing_status ON TABLE processing_lock TYPE string;
        
        DEFINE INDEX idx_processing_lock_repo ON TABLE processing_lock COLUMNS repo_id UNIQUE;
    "#)
    .await?;
    
    Ok(())
}

/// Helper function to clean up test database
async fn cleanup_test_db(db: &Surreal<Client>) -> Result<()> {
    db.query("DELETE repo").await?;
    db.query("DELETE processing_lock").await?;
    Ok(())
}