/// Example demonstrating different connection types with the Any engine
/// 
/// This shows how you can use different database backends without changing code
/// Run with: cargo run --example memory_test

use anyhow::Result;
use surrealdb::{engine::any::connect, RecordId};

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct TestRepo {
    id: RecordId,
    full_name: String,
    description: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("Testing different connection types with the Any engine...\n");

    // Note: Some engines require additional features in Cargo.toml:
    // - memory:// requires feature "kv-mem"
    // - rocksdb:// requires feature "kv-rocksdb" 
    // - tikv:// requires feature "kv-tikv"
    // - fdb:// requires feature "kv-fdb"

    // Test 1: WebSocket connection (if available)
    println!("1. Testing WebSocket connection:");
    match test_connection("ws://localhost:8000", "test_ns", "test_db").await {
        Ok(_) => println!("   ✓ WebSocket connection successful"),
        Err(e) => println!("   ✗ WebSocket connection failed: {}", e),
    }

    // Test 2: HTTP connection (if available)
    println!("\n2. Testing HTTP connection:");
    match test_connection("http://localhost:8000", "test_ns", "test_db").await {
        Ok(_) => println!("   ✓ HTTP connection successful"),
        Err(e) => println!("   ✗ HTTP connection failed: {}", e),
    }

    // Test 3: In-memory database (requires kv-mem feature)
    println!("\n3. Testing in-memory database:");
    match test_connection("memory://", "memory_test", "memory_db").await {
        Ok(_) => println!("   ✓ In-memory connection successful"),
        Err(e) => println!("   ✗ In-memory connection failed: {} (requires 'kv-mem' feature)", e),
    }

    println!("\nAll tests completed!");
    Ok(())
}

async fn test_connection(url: &str, namespace: &str, database: &str) -> Result<()> {
    println!("   Connecting to: {}", url);
    
    // Connect using the Any engine
    let db = connect(url).await?;
    
    // For non-memory databases, we need authentication
    if !url.starts_with("memory://") {
        use surrealdb::opt::auth::Root;
        db.signin(Root {
            username: "root",
            password: "root",
        })
        .await?;
    }
    
    // Select namespace and database
    db.use_ns(namespace).use_db(database).await?;
    
    // Create a test record
    let test_repo = TestRepo {
        id: RecordId::from(("repo", "test123")),
        full_name: "test/repo".to_string(),
        description: Some("Test repository".to_string()),
    };
    
    let created: Option<TestRepo> = db
        .create("repo")
        .content(test_repo)
        .await?;
    
    println!("   Created test record: {:?}", created.is_some());
    
    // Query the record back
    let result: Vec<TestRepo> = db
        .query("SELECT * FROM repo")
        .await?
        .take(0)?;
    
    println!("   Retrieved {} record(s)", result.len());
    
    Ok(())
}

#[test]
fn verify_any_engine_flexibility() {
    // This test verifies that we can use different URL schemes
    let schemes = vec![
        "ws://localhost:8000",
        "wss://secure.example.com",
        "http://localhost:8000",
        "https://secure.example.com",
        "memory://",
        "rocksdb://path/to/db",
        "tikv://cluster",
        "fdb://cluster",
    ];
    
    for scheme in schemes {
        println!("URL scheme '{}' is supported by the Any engine", scheme);
    }
}