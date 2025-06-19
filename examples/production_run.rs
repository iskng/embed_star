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
use surrealdb::{ engine::any::{ Any, connect }, opt::auth::Root, RecordId, Surreal, sql::Datetime };
use std::time::Duration;
use tokio::time::sleep;
use tracing::{ info, warn };
use tracing_subscriber::{ layer::SubscriberExt, util::SubscriberInitExt, EnvFilter };

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct TestRepo {
    id: RecordId,
    github_id: i64,
    name: String,
    full_name: String,
    description: Option<String>,
    url: String,
    stars: u32,
    language: Option<String>,
    owner: RepoOwner,
    is_private: bool,
    created_at: Datetime,
    updated_at: Datetime,
    embedding: Option<Vec<f32>>,
    embedding_generated_at: Option<Datetime>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct RepoOwner {
    login: String,
    avatar_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber
        ::registry()
        .with(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| { "info,embed_star=debug".into() })
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting production run example");
    dotenv::dotenv().ok();
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
    // Force test database for this example
    std::env::set_var("DB_NAMESPACE", "test");
    std::env::set_var("DB_DATABASE", "embed_star_example");

    // Use embedding provider from .env file (already loaded by dotenv)
    if std::env::var("EMBEDDING_PROVIDER").is_err() {
        warn!("EMBEDDING_PROVIDER not set, please configure in .env file");
        std::env::set_var("EMBEDDING_PROVIDER", "together");
    }

    // Create database connection - use the same database as the service
    let db_url = std::env::var("DB_URL").unwrap_or_else(|_| "ws://localhost:8000".to_string());
    let db_user = std::env::var("DB_USER").unwrap_or_else(|_| "root".to_string());
    let db_pass = std::env::var("DB_PASS").unwrap_or_else(|_| "root".to_string());
    let db_namespace = std::env::var("DB_NAMESPACE").unwrap_or_else(|_| "test".to_string());
    let db_database = std::env::var("DB_DATABASE").unwrap_or_else(|_| "embed_star_example".to_string());

    info!("Connecting to database at {} ({}/{})", db_url, db_namespace, db_database);
    let db: Surreal<Any> = connect(&db_url).await?;
    db.signin(Root {
        username: &db_user,
        password: &db_pass,
    }).await?;
    db.use_ns(&db_namespace).use_db(&db_database).await?;

    // Create repo table - use SCHEMALESS for flexibility
    info!("Setting up database schema");
    match db.query("INFO FOR TABLE repo").await {
        Ok(_) => info!("Table 'repo' already exists"),
        Err(_) => {
            let _ = db.query("DEFINE TABLE repo SCHEMALESS").await?;
            info!("Created table 'repo'");
        }
    }


    // Clean up any existing test repositories first
    info!("Cleaning up existing test repositories");
    let test_repos = vec![
        "rust-lang/rust", "facebook/react", "golang/go", "python/cpython", 
        "torvalds/linux", "kubernetes/kubernetes", "tensorflow/tensorflow",
        "microsoft/vscode", "docker/docker", "elastic/elasticsearch"
    ];
    
    for full_name in &test_repos {
        let repo_id = full_name.replace("/", "_").replace("-", "_");
        let _: Result<Option<TestRepo>, _> = db.delete(("repo", repo_id.as_str())).await;
    }
    
    // Create sample repositories
    info!("Creating sample repositories");
    let sample_repos = vec![
        (1, "rust-lang/rust", "The Rust programming language", "Rust", 90000),
        (
            2,
            "facebook/react",
            "A declarative, efficient, and flexible JavaScript library",
            "JavaScript",
            220000,
        ),
        (3, "golang/go", "The Go programming language", "Go", 120000),
        (4, "python/cpython", "The Python programming language", "C", 60000),
        (5, "torvalds/linux", "Linux kernel source tree", "C", 170000),
        (
            6,
            "kubernetes/kubernetes",
            "Production-Grade Container Scheduling and Management",
            "Go",
            105000,
        ),
        (7, "tensorflow/tensorflow", "An Open Source Machine Learning Framework", "C++", 180000),
        (8, "microsoft/vscode", "Visual Studio Code", "TypeScript", 155000),
        (9, "docker/docker", "Docker - the open-source application container engine", "Go", 68000),
        (
            10,
            "elastic/elasticsearch",
            "Free and Open, Distributed, RESTful Search Engine",
            "Java",
            67000,
        )
    ];

    for (github_id, full_name, description, language, stars) in sample_repos {
        let parts: Vec<&str> = full_name.split('/').collect();
        let owner_login = parts[0];
        let name = parts[1];

        // Create repo using the create method with proper RecordId
        let repo_id = full_name.replace("/", "_").replace("-", "_");
        
        let test_repo = TestRepo {
            id: RecordId::from(("repo", repo_id.as_str())),
            github_id,
            name: name.to_string(),
            full_name: full_name.to_string(),
            description: Some(description.to_string()),
            url: format!("https://github.com/{}", full_name),
            stars,
            language: Some(language.to_string()),
            owner: RepoOwner {
                login: owner_login.to_string(),
                avatar_url: format!("https://github.com/{}.png", owner_login),
            },
            is_private: false,
            created_at: Datetime::from(chrono::Utc::now() - chrono::Duration::days(1)),
            updated_at: Datetime::from(chrono::Utc::now()),
            embedding: None,
            embedding_generated_at: None,
        };

        match db.create(("repo", repo_id.as_str())).content(test_repo).await {
            Ok::<Option<TestRepo>, _>(result) => {
                if result.is_some() {
                    info!("Created repo: {}", full_name);
                } else {
                    warn!("Failed to create repo {}: no result returned", full_name);
                }
            }
            Err(e) => {
                warn!("Failed to create repo {}: {}", full_name, e);
            }
        }
    }

    info!("Created sample repositories");
    
    // Verify creation by listing repos
    let mut list_response = db.query("SELECT full_name FROM repo LIMIT 10").await?;
    let list_repos: Vec<serde_json::Value> = list_response.take(0)?;
    info!("After creation, found {} repos in database", list_repos.len());
    for repo in &list_repos {
        if let Some(name) = repo.get("full_name").and_then(|v| v.as_str()) {
            info!("  - Found repo: {}", name);
        }
    }

    // Check initial state
    let mut total_response = db.query("SELECT count() FROM repo GROUP ALL").await?;
    let total_val: Option<serde_json::Value> = total_response.take(0)?;
    let total_repos = match total_val {
        Some(val) => {
            if let Some(count) = val.get("count").and_then(|v| v.as_i64()) {
                count as usize
            } else {
                0
            }
        }
        None => 0
    };

    let mut without_response = db
        .query("SELECT count() FROM repo WHERE embedding IS NONE GROUP ALL").await?;
    let without_val: Option<serde_json::Value> = without_response.take(0)?;
    let repos_without_embeddings = match without_val {
        Some(val) => {
            if let Some(count) = val.get("count").and_then(|v| v.as_i64()) {
                count as usize
            } else {
                0
            }
        }
        None => 0
    };

    info!(
        "Database state: {} total repos, {} without embeddings",
        total_repos,
        repos_without_embeddings
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

        let mut embedded_response = monitor_db
            .query("SELECT count() FROM repo WHERE embedding IS NOT NONE GROUP ALL").await?;
        let embedded_val: Option<serde_json::Value> = embedded_response.take(0)?;
        let embedded = match embedded_val {
            Some(val) => {
                if let Some(count) = val.get("count").and_then(|v| v.as_i64()) {
                    count as usize
                } else {
                    0
                }
            }
            None => 0
        };

        info!("Progress: {}/{} repositories have embeddings", embedded, total_repos);

        if embedded == total_repos && embedded > 0 {
            info!("All repositories have embeddings!");
            break;
        }

        if i == 29 {
            warn!("Timeout: Not all repositories were embedded in 60 seconds");
        }
    }

    // Verify embeddings
    info!("Verifying embeddings...");
    let mut verify_response = db.query("SELECT * FROM repo").await?;
    let repos: Vec<TestRepo> = verify_response.take(0)?;

    let mut stats = EmbeddingStats::default();

    for repo in &repos {
        if let Some(embedding) = &repo.embedding {
            stats.total_with_embeddings += 1;
            stats.total_dimensions += embedding.len();
            // Model is now configured at service level, not per-repo

            // Check embedding quality
            let magnitude: f32 = embedding
                .iter()
                .map(|x| x * x)
                .sum::<f32>()
                .sqrt();
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
    info!("  Model used: {}", std::env::var("EMBEDDING_MODEL").unwrap_or_else(|_| "unknown".to_string()));

    // Sample similarity search
    if stats.total_with_embeddings > 0 {
        info!("Testing similarity search...");

        // Get embedding for rust repo
        let rust_repo_id = "rust_lang_rust";
        let rust_repo: Option<TestRepo> = db
            .select(("repo", rust_repo_id))
            .await?;

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

    info!("Production run example completed successfully!");
    Ok(())
}

#[derive(Default)]
struct EmbeddingStats {
    total_with_embeddings: usize,
    valid_embeddings: usize,
    total_dimensions: usize,
}

async fn find_similar_repos(
    db: &Surreal<Any>,
    target_embedding: &[f32],
    limit: usize
) -> Result<Vec<(String, f32)>> {
    let mut response = db
        .query("SELECT * FROM repo WHERE embedding IS NOT NULL").await?;
    let repos: Vec<TestRepo> = response.take(0)?;

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

    let dot_product: f32 = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| x * y)
        .sum();
    let magnitude_a: f32 = a
        .iter()
        .map(|x| x * x)
        .sum::<f32>()
        .sqrt();
    let magnitude_b: f32 = b
        .iter()
        .map(|x| x * x)
        .sum::<f32>()
        .sqrt();

    if magnitude_a == 0.0 || magnitude_b == 0.0 {
        0.0
    } else {
        dot_product / (magnitude_a * magnitude_b)
    }
}
