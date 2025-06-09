# embed_star

A Rust service that generates embeddings for GitHub repository data stored in SurrealDB.

## Overview

This service:
- Connects to a SurrealDB instance containing GitHub repository data
- Monitors for repositories that need embeddings (new or updated)
- Generates embeddings using Ollama (local), OpenAI, or Together AI (cloud)
- Updates the repository records with embeddings for similarity search

## Prerequisites

1. SurrealDB running with the `gitstars` namespace and `stars` database
2. One of the following embedding providers:
   - Ollama running locally with an embedding model (e.g., `nomic-embed-text`)
   - OpenAI API key for cloud embeddings
   - Together AI API key for cloud embeddings

## Database Setup

Before running, add the embedding fields to your repo table:

```sql
DEFINE FIELD embedding ON TABLE repo TYPE option<array<float>>;
DEFINE FIELD embedding_model ON TABLE repo TYPE option<string>;
DEFINE FIELD embedding_generated_at ON TABLE repo TYPE option<datetime>;
```

## Configuration

Create a `.env` file (see `.env.example`):

```bash
# Database
DB_URL=ws://localhost:8000
DB_USER=root
DB_PASS=root
DB_NAMESPACE=gitstars
DB_DATABASE=stars

# Embeddings (Ollama)
EMBEDDING_PROVIDER=ollama
OLLAMA_URL=http://localhost:11434
EMBEDDING_MODEL=nomic-embed-text

# OR for OpenAI:
# EMBEDDING_PROVIDER=openai
# OPENAI_API_KEY=sk-...
# EMBEDDING_MODEL=text-embedding-3-small

# OR for Together AI:
# EMBEDDING_PROVIDER=together
# TOGETHER_API_KEY=your-together-api-key
# EMBEDDING_MODEL=togethercomputer/m2-bert-80M-8k-retrieval

# Processing
BATCH_SIZE=10
```

## Usage

```bash
# Install dependencies and build
cargo build --release

# Run with environment variables
cargo run --release

# Or with custom settings
cargo run --release -- \
  --db-url ws://localhost:8000 \
  --embedding-provider ollama \
  --embedding-model nomic-embed-text
```

## How It Works

1. **Initial Processing**: On startup, processes all existing repos without embeddings
2. **Live Monitoring**: Continuously polls for new or updated repositories
3. **Batch Processing**: Processes repositories in configurable batches for efficiency
4. **Retry Logic**: Automatically retries failed embeddings with exponential backoff

## Embedding Content

For each repository, the following data is combined for embedding:
- Repository full name (owner/name)
- Description (if available)  
- Primary programming language
- Star count
- Owner login

## Supported Embedding Providers

### Ollama (Local)
- Default provider for local deployments
- Popular models: `nomic-embed-text`, `mxbai-embed-large`
- No API costs, runs on your hardware

### OpenAI
- High-quality embeddings with `text-embedding-3-small` or `text-embedding-3-large`
- Requires OpenAI API key
- Pricing based on token usage

### Together AI
- Cost-effective cloud embeddings
- Recommended models: `togethercomputer/m2-bert-80M-8k-retrieval`, `BAAI/bge-base-en-v1.5`
- Lower cost than OpenAI with good performance
- Requires Together AI API key

## Architecture

- Uses connection pooling for database efficiency
- Supports multiple embedding providers through a trait-based design
- Implements concurrent processing with controlled parallelism
- Provides detailed logging for monitoring

## Logging

Set log level via environment variable:

```bash
RUST_LOG=warn,embed_star=info cargo run
```

## Performance Tuning

- `BATCH_SIZE`: Number of repos to process in parallel
- `POOL_SIZE`: Database connection pool size
- `BATCH_DELAY_MS`: Delay between batches to avoid overload
- `RETRY_ATTEMPTS`: Number of retries for failed embeddings

## Production Deployment

### Health Monitoring

The service exposes the following endpoints on port 9090:
- `/health` - Health check endpoint with database connectivity status
- `/metrics` - Prometheus metrics endpoint
- `/livez` - Kubernetes liveness probe endpoint

### Metrics

Key metrics exposed:
- `embed_star_embeddings_total` - Total embeddings generated
- `embed_star_embeddings_errors_total` - Total embedding errors
- `embed_star_embedding_duration_seconds` - Embedding generation time
- `embed_star_repos_pending` - Number of repos pending embeddings
- `embed_star_rate_limits_total` - Rate limit hits by provider

### Docker Deployment

```bash
# Build the image
docker build -t embed_star:latest .

# Run with environment variables
docker run -d \
  --name embed_star \
  -p 9090:9090 \
  --env-file .env.production \
  embed_star:latest
```

### Docker Compose

For a complete stack with monitoring:

```bash
docker-compose up -d
```

This starts:
- embed_star service
- SurrealDB
- Ollama (for local embeddings)
- Prometheus (metrics collection)
- Grafana (visualization on port 3000)

### Kubernetes Deployment

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: embed-star
spec:
  replicas: 3
  selector:
    matchLabels:
      app: embed-star
  template:
    metadata:
      labels:
        app: embed-star
    spec:
      containers:
      - name: embed-star
        image: your-registry/embed_star:latest
        ports:
        - containerPort: 9090
          name: metrics
        env:
        - name: DB_URL
          value: "ws://surrealdb:8000"
        - name: EMBEDDING_PROVIDER
          value: "together"
        envFrom:
        - secretRef:
            name: embed-star-secrets
        livenessProbe:
          httpGet:
            path: /livez
            port: 9090
          initialDelaySeconds: 5
          periodSeconds: 10
        readinessProbe:
          httpGet:
            path: /health
            port: 9090
          initialDelaySeconds: 10
          periodSeconds: 5
        resources:
          requests:
            memory: "256Mi"
            cpu: "100m"
          limits:
            memory: "512Mi"
            cpu: "500m"
```

### Production Best Practices

1. **Security**
   - Store API keys in secrets management (Kubernetes Secrets, AWS Secrets Manager, etc.)
   - Run as non-root user (already configured in Dockerfile)
   - Use TLS for database connections in production

2. **Reliability**
   - Deploy multiple replicas for high availability
   - Configure appropriate resource limits
   - Use persistent storage for SurrealDB
   - Set up alerting on key metrics

3. **Performance**
   - Tune batch size based on your workload
   - Monitor rate limits and adjust accordingly
   - Use local Ollama for cost-effective embeddings at scale
   - Consider regional deployments near your database

4. **Observability**
   - Export logs to centralized logging (ELK, Datadog, etc.)
   - Set up dashboards for key metrics
   - Configure alerts for error rates and processing delays
   - Use distributed tracing for debugging

### CI/CD

GitHub Actions workflows are included for:
- Continuous Integration (lint, test, build)
- Security audits
- Docker image building
- Release automation with multi-platform binaries

To create a release:
```bash
git tag -a v1.0.0 -m "Release v1.0.0"
git push origin v1.0.0
```