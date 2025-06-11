# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

embed_star is a production-ready Rust service that generates embeddings for GitHub repository data stored in SurrealDB. It supports multiple embedding providers (Ollama, OpenAI, Together AI) and is designed for high-scale deployments with comprehensive monitoring and error handling.

## Key Commands

### Development
```bash
# Build and run in development
cargo run

# Build for production
cargo build --release

# Run tests
cargo test

# Run a specific test
cargo test test_repo_needs_embedding

# Check code without building
cargo check

# Format code
cargo fmt

# Lint code
cargo clippy -- -D warnings

# Run with custom log level
RUST_LOG=debug,embed_star=trace cargo run

# Run with specific embedding provider
cargo run -- --embedding-provider together --embedding-model togethercomputer/m2-bert-80M-8k-retrieval
```

### Docker Operations
```bash
# Build Docker image
docker build -t embed_star:latest .

# Run complete stack with monitoring
docker-compose up -d

# View logs
docker-compose logs -f embed_star
```

### Database Migrations
The service automatically runs migrations on startup. Manual migration commands are not typically needed, but the migration system is in `src/migration.rs`.

## Architecture Overview

### Core Processing Flow
1. **main.rs**: Entry point that orchestrates all components:
   - Sets up database connection pool
   - Runs migrations automatically
   - Spawns concurrent tasks for initial batch processing, live query monitoring, and metrics reporting
   - Implements graceful shutdown handling

2. **surreal_client.rs**: Database interface layer:
   - Uses polling instead of live queries (compatibility with SurrealDB v1.5)
   - Tracks processed repositories to avoid duplicates
   - Provides methods for fetching repos needing embeddings and updating them

3. **embedder.rs**: Embedding generation abstraction:
   - Trait-based design (`EmbeddingProvider`) for multiple providers
   - Implementations for Ollama (local), OpenAI, and Together AI
   - Each provider handles its own API specifics and error cases

### Production Features

1. **Error Handling (error.rs)**:
   - Custom error types with `thiserror`
   - Distinguishes retryable vs non-retryable errors
   - Error codes for metrics tracking

2. **Retry Logic (retry.rs)**:
   - Exponential backoff implementation
   - Configurable retry attempts and delays
   - Only retries errors marked as retryable

3. **Rate Limiting (rate_limiter.rs)**:
   - Per-provider rate limits (OpenAI: 3000/min, Together: 1000/min)
   - Uses `governor` crate for token bucket implementation
   - Prevents API throttling

4. **Metrics (metrics.rs, server.rs)**:
   - Prometheus metrics exposed on port 9090
   - Health check endpoints for Kubernetes
   - Key metrics: embeddings_total, errors, duration, pending repos

5. **Graceful Shutdown (shutdown.rs)**:
   - Handles SIGINT/SIGTERM signals
   - Waits for all tasks to complete
   - Configurable shutdown timeout

### Key Design Decisions

1. **Polling vs Live Queries**: Due to SurrealDB v1.5 API changes, the service uses polling with deduplication instead of live queries. The polling interval is 5 seconds.

2. **Connection Pooling**: Simplified to use `Arc<Surreal<Client>>` instead of deadpool due to version compatibility issues.

3. **Batch Processing**: Repos are processed in configurable batches (default 10) with delays between batches to prevent overload.

4. **Embedding Text Format**: Combines repo name, description, language, star count, and owner into a structured text format for embedding.

## Environment Configuration

Critical environment variables:
- `DB_URL`: SurrealDB WebSocket URL (default: ws://localhost:8000)
- `EMBEDDING_PROVIDER`: Choice of ollama, openai, or together
- `OPENAI_API_KEY` / `TOGETHER_API_KEY`: Required for cloud providers
- `EMBEDDING_MODEL`: Model name specific to chosen provider
- `BATCH_SIZE`: Number of repos to process concurrently
- `MONITORING_PORT`: Port for metrics/health endpoints (default: 9090)

## Database Schema

The service expects a `repo` table with these fields and adds embedding fields:
```sql
-- Added by migrations
DEFINE FIELD embedding ON TABLE repo TYPE option<array<float>>;
DEFINE FIELD embedding_model ON TABLE repo TYPE option<string>;
DEFINE FIELD embedding_generated_at ON TABLE repo TYPE option<datetime>;
```

## Testing Approach

Tests are in `tests/integration_tests.rs` and focus on:
- Repository embedding logic (needs_embedding, prepare_text_for_embedding)
- Error retry behavior
- Configuration validation

Individual module tests use `#[cfg(test)]` blocks within their files.

## Monitoring Integration

The service exposes:
- `/health` - Database connectivity check
- `/metrics` - Prometheus metrics
- `/livez` - Simple liveness check

Metrics are designed for Grafana dashboards and alerting.

## Performance Considerations

1. Rate limits are enforced per provider to avoid API throttling
2. Batch processing reduces database round trips
3. Exponential backoff prevents thundering herd on errors
4. Connection pooling would improve under high load (currently simplified)

## Common Issues and Solutions

1. **SurrealDB Connection**: Ensure SurrealDB is running and accessible at the configured URL
2. **Rate Limits**: If hitting rate limits, reduce BATCH_SIZE or increase BATCH_DELAY_MS
3. **Memory Usage**: Large batches can consume significant memory; tune BATCH_SIZE accordingly
4. **Embedding Provider Errors**: Check API keys and network connectivity to providers