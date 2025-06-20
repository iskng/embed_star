use reqwest;
use serde_json::Value;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing health check endpoint...");
    
    // Make request to health endpoint
    let response = reqwest::get("http://localhost:9090/health")
        .await?;
    
    println!("Status: {}", response.status());
    
    let body: Value = response.json().await?;
    println!("Response: {}", serde_json::to_string_pretty(&body)?);
    
    // Check specific fields
    if let Some(providers) = body.get("embedding_providers").and_then(|v| v.as_array()) {
        println!("\nEmbedding Providers:");
        for provider in providers {
            println!("  - Name: {}", provider.get("name").and_then(|v| v.as_str()).unwrap_or("unknown"));
            println!("    Available: {}", provider.get("available").and_then(|v| v.as_bool()).unwrap_or(false));
            if let Some(latency) = provider.get("latency_ms").and_then(|v| v.as_u64()) {
                println!("    Latency: {}ms", latency);
            }
        }
    }
    
    Ok(())
}