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