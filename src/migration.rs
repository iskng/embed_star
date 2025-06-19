use crate::pool::Pool;
use anyhow::Result;
use tracing::{info, warn};

pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub up: &'static str,
    pub down: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "add_embedding_fields",
        up: r#"
            DEFINE FIELD IF NOT EXISTS embedding ON TABLE repo TYPE option<array<float>>;
            DEFINE FIELD IF NOT EXISTS embedding_generated_at ON TABLE repo TYPE option<datetime>;
        "#,
        down: r#"
            REMOVE FIELD embedding ON TABLE repo;
            REMOVE FIELD embedding_generated_at ON TABLE repo;
        "#,
    },
    Migration {
        version: 2,
        name: "add_embedding_indexes",
        up: r#"
            DEFINE INDEX IF NOT EXISTS idx_repo_embedding_generated_at ON TABLE repo COLUMNS embedding_generated_at;
        "#,
        down: r#"
            REMOVE INDEX idx_repo_embedding_generated_at ON TABLE repo;
        "#,
    },
];

pub async fn run_migrations(pool: &Pool) -> Result<()> {
    info!("Running database migrations...");
    
    // Get a connection from the pool
    let db = pool.get().await
        .map_err(|e| anyhow::anyhow!("Failed to get connection from pool: {}", e))?;
    
    // Create migration tracking table
    db.query(r#"
        DEFINE TABLE IF NOT EXISTS migration SCHEMAFULL;
        DEFINE FIELD version ON TABLE migration TYPE int;
        DEFINE FIELD name ON TABLE migration TYPE string;
        DEFINE FIELD applied_at ON TABLE migration TYPE datetime;
        DEFINE INDEX idx_migration_version ON TABLE migration COLUMNS version UNIQUE;
    "#)
    .await?;
    
    // Get current version
    let mut response = db
        .query("SELECT VALUE version FROM migration ORDER BY version DESC LIMIT 1")
        .await?;
    let current_version: Option<u32> = response.take(0)?;
    
    let current_version = current_version.unwrap_or(0);
    info!("Current migration version: {}", current_version);
    
    // Apply pending migrations
    let pending_migrations: Vec<&Migration> = MIGRATIONS
        .iter()
        .filter(|m| m.version > current_version)
        .collect();
    
    if pending_migrations.is_empty() {
        info!("No pending migrations");
        return Ok(());
    }
    
    for migration in pending_migrations {
        info!("Applying migration {}: {}", migration.version, migration.name);
        
        // Begin transaction
        db.query("BEGIN TRANSACTION").await?;
        
        match db.query(migration.up).await {
            Ok(_) => {
                // Record migration
                db.query(
                    "CREATE migration CONTENT {
                        version: $version,
                        name: $name,
                        applied_at: time::now()
                    }"
                )
                .bind(("version", migration.version))
                .bind(("name", migration.name.to_string()))
                .await?;
                
                db.query("COMMIT TRANSACTION").await?;
                info!("Migration {} applied successfully", migration.version);
            }
            Err(e) => {
                db.query("CANCEL TRANSACTION").await?;
                return Err(anyhow::anyhow!(
                    "Failed to apply migration {}: {}",
                    migration.version,
                    e
                ));
            }
        }
    }
    
    info!("All migrations completed successfully");
    Ok(())
}

pub async fn rollback_migration(pool: &Pool, target_version: u32) -> Result<()> {
    // Get a connection from the pool
    let db = pool.get().await
        .map_err(|e| anyhow::anyhow!("Failed to get connection from pool: {}", e))?;
    
    let mut response = db
        .query("SELECT VALUE version FROM migration ORDER BY version DESC LIMIT 1")
        .await?;
    let current_version: Option<u32> = response.take(0)?;
    
    let current_version = current_version.unwrap_or(0);
    
    if target_version >= current_version {
        warn!("Target version {} is not less than current version {}", target_version, current_version);
        return Ok(());
    }
    
    let migrations_to_rollback: Vec<&Migration> = MIGRATIONS
        .iter()
        .filter(|m| m.version > target_version && m.version <= current_version)
        .rev()
        .collect();
    
    for migration in migrations_to_rollback {
        info!("Rolling back migration {}: {}", migration.version, migration.name);
        
        db.query("BEGIN TRANSACTION").await?;
        
        match db.query(migration.down).await {
            Ok(_) => {
                db.query("DELETE migration WHERE version = $version")
                    .bind(("version", migration.version))
                    .await?;
                
                db.query("COMMIT TRANSACTION").await?;
                info!("Migration {} rolled back successfully", migration.version);
            }
            Err(e) => {
                db.query("CANCEL TRANSACTION").await?;
                return Err(anyhow::anyhow!(
                    "Failed to rollback migration {}: {}",
                    migration.version,
                    e
                ));
            }
        }
    }
    
    Ok(())
}