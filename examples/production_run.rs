/// Example demonstrating a full production run of the embed_star service
/// 
/// This example shows how to:
/// 1. Set up a test database with sample repositories
/// 2. Run the embedding service
/// 3. Verify embeddings are generated
/// 4. Monitor performance metrics
///
/// Run with: cargo run --example production_run

use anyhow::Result;
use surrealdb::{
    engine::remote::ws::{Client, Ws},
    opt::auth::Root,
    sql::Thing,
    Surreal,
};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

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

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            "info,embed_star=debug".into()
        }))
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting production run example");

    // Check if we have required environment variables
    if std::env::var("DB_URL").is_err() {
        warn!("DB_URL not set, using default ws://localhost:8000");
        std::env::set_var("DB_URL", "ws://localhost:8000");
    }
    if std::env::var("DB_USER").is_err() {
        std::env::set_var("DB_USER", "root");
    }
    if std::env::var("DB_PASS").is_err() {
        std::env::set_var("DB_PASS", "root");
    }
    if std::env::var("DB_NAMESPACE").is_err() {
        std::env::set_var("DB_NAMESPACE", "test");
    }
    if std::env::var("DB_DATABASE").is_err() {
        std::env::set_var("DB_DATABASE", "embed_star_example");
    }

    // Set embedding provider (use Ollama for local testing)
    if std::env::var("EMBEDDING_PROVIDER").is_err() {
        warn!("EMBEDDING_PROVIDER not set, using ollama");
        std::env::set_var("EMBEDDING_PROVIDER", "ollama");
        std::env::set_var("OLLAMA_URL", "http://localhost:11434");
        std::env::set_var("EMBEDDING_MODEL", "nomic-embed-text");
    }

    // Create database connection
    let db_url = std::env::var("DB_URL")?;
    let db_user = std::env::var("DB_USER")?;
    let db_pass = std::env::var("DB_PASS")?;
    let db_namespace = std::env::var("DB_NAMESPACE")?;
    let db_database = std::env::var("DB_DATABASE")?;

    info!("Connecting to database at {}", db_url);
    let db = Surreal::new::<Ws>(&db_url).await?;
    db.signin(Root {
        username: &db_user,
        password: &db_pass,
    })
    .await?;
    db.use_ns(&db_namespace).use_db(&db_database).await?;

    // Create repo table with required fields
    info!("Setting up database schema");
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
        DEFINE INDEX idx_repo_embedding ON TABLE repo COLUMNS embedding_generated_at;
    "#)
    .await?;

    // Create processing_lock table for deduplication
    db.query(r#"
        DEFINE TABLE processing_lock SCHEMAFULL;
        DEFINE FIELD repo_id ON TABLE processing_lock TYPE record<repo>;
        DEFINE FIELD instance_id ON TABLE processing_lock TYPE string;
        DEFINE FIELD locked_at ON TABLE processing_lock TYPE datetime;
        DEFINE FIELD expires_at ON TABLE processing_lock TYPE datetime;
        DEFINE FIELD processing_status ON TABLE processing_lock TYPE string;
        
        DEFINE INDEX idx_processing_lock_repo ON TABLE processing_lock COLUMNS repo_id UNIQUE;
    "#)
    .await?;

    // Create sample repositories
    info!("Creating sample repositories");
    let sample_repos = vec![
        ("rust-lang/rust", "The Rust programming language", "Rust", 90000),
        ("facebook/react", "A declarative, efficient, and flexible JavaScript library", "JavaScript", 220000),
        ("golang/go", "The Go programming language", "Go", 120000),
        ("python/cpython", "The Python programming language", "C", 60000),
        ("torvalds/linux", "Linux kernel source tree", "C", 170000),
        ("kubernetes/kubernetes", "Production-Grade Container Scheduling and Management", "Go", 105000),
        ("tensorflow/tensorflow", "An Open Source Machine Learning Framework", "C++", 180000),
        ("microsoft/vscode", "Visual Studio Code", "TypeScript", 155000),
        ("docker/docker", "Docker - the open-source application container engine", "Go", 68000),
        ("elastic/elasticsearch", "Free and Open, Distributed, RESTful Search Engine", "Java", 67000),
    ];

    for (full_name, description, language, stars) in sample_repos {
        let owner_login = full_name.split('/').next().unwrap();
        
        let query = r#"
            CREATE repo CONTENT {
                full_name: $full_name,
                description: $description,
                language: $language,
                stars: $stars,
                owner_login: $owner_login,
                created_at: time::now() - 1d,
                updated_at: time::now()
            }
        "#;
        
        db.query(query)
            .bind(("full_name", full_name))
            .bind(("description", description))
            .bind(("language", language))
            .bind(("stars", stars))
            .bind(("owner_login", owner_login))
            .await?;
    }

    info!("Created sample repositories");

    // Check initial state
    let total_repos: Option<usize> = db
        .query("SELECT count() FROM repo GROUP ALL")
        .await?
        .take(0)?;
    
    let repos_without_embeddings: Option<usize> = db
        .query("SELECT count() FROM repo WHERE embedding IS NULL GROUP ALL")
        .await?
        .take(0)?;

    info!(
        "Database state: {} total repos, {} without embeddings",
        total_repos.unwrap_or(0),
        repos_without_embeddings.unwrap_or(0)
    );

    // Start the embedding service in a separate task
    info!("Starting embed_star service");
    let service_handle = tokio::spawn(async move {
        // Run the actual service
        match embed_star::run_service().await {
            Ok(_) => info!("Service completed successfully"),
            Err(e) => warn!("Service error: {}", e),
        }
    });

    // Monitor progress
    info!("Monitoring embedding generation progress...");
    let monitor_db = db.clone();
    
    for i in 0..30 {
        sleep(Duration::from_secs(2)).await;
        
        let embedded_count: Option<usize> = monitor_db
            .query("SELECT count() FROM repo WHERE embedding IS NOT NULL GROUP ALL")
            .await?
            .take(0)?;
        
        let embedded = embedded_count.unwrap_or(0);
        info!(
            "Progress: {}/{} repositories have embeddings",
            embedded,
            total_repos.unwrap_or(0)
        );
        
        if embedded == total_repos.unwrap_or(0) && embedded > 0 {
            info!("All repositories have embeddings!");
            break;
        }
        
        if i == 29 {
            warn!("Timeout: Not all repositories were embedded in 60 seconds");
        }
    }

    // Verify embeddings
    info!("Verifying embeddings...");
    let repos: Vec<TestRepo> = db.query("SELECT * FROM repo").await?.take(0)?;
    
    let mut stats = EmbeddingStats::default();
    
    for repo in &repos {
        if let Some(embedding) = &repo.embedding {
            stats.total_with_embeddings += 1;
            stats.total_dimensions += embedding.len();
            stats.models.insert(repo.embedding_model.clone().unwrap_or_default());
            
            // Check embedding quality
            let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
            if magnitude > 0.1 {
                stats.valid_embeddings += 1;
            }
        }
    }
    
    info!("Embedding statistics:");
    info!("  Total repositories: {}", repos.len());
    info!("  With embeddings: {}", stats.total_with_embeddings);
    info!("  Valid embeddings: {}", stats.valid_embeddings);
    info!("  Average dimensions: {}", stats.total_dimensions / stats.total_with_embeddings.max(1));
    info!("  Models used: {:?}", stats.models);

    // Sample similarity search
    if stats.total_with_embeddings > 0 {
        info!("Testing similarity search...");
        
        // Get embedding for rust repo
        let rust_repo: Option<TestRepo> = db
            .query("SELECT * FROM repo WHERE full_name = 'rust-lang/rust'")
            .await?
            .take(0)?;
        
        if let Some(repo) = rust_repo {
            if let Some(embedding) = repo.embedding {
                // Find similar repos (simple cosine similarity)
                let similar = find_similar_repos(&db, &embedding, 3).await?;
                info!("Repositories similar to rust-lang/rust:");
                for (name, score) in similar {
                    info!("  {} (similarity: {:.3})", name, score);
                }
            }
        }
    }

    // Cleanup
    info!("Cleaning up...");
    
    // Send shutdown signal to service
    service_handle.abort();
    
    // Clean up test data (optional)
    // db.query("DELETE repo").await?;
    // db.query("DELETE processing_lock").await?;
    
    info!("Production run example completed successfully!");
    Ok(())
}

#[derive(Default)]
struct EmbeddingStats {
    total_with_embeddings: usize,
    valid_embeddings: usize,
    total_dimensions: usize,
    models: std::collections::HashSet<String>,
}

async fn find_similar_repos(
    db: &Surreal<Client>,
    target_embedding: &[f32],
    limit: usize,
) -> Result<Vec<(String, f32)>> {
    let repos: Vec<TestRepo> = db.query("SELECT * FROM repo WHERE embedding IS NOT NULL").await?.take(0)?;
    
    let mut similarities = Vec::new();
    
    for repo in repos {
        if let Some(embedding) = repo.embedding {
            let similarity = cosine_similarity(target_embedding, &embedding);
            similarities.push((repo.full_name, similarity));
        }
    }
    
    similarities.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    similarities.truncate(limit);
    
    Ok(similarities)
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