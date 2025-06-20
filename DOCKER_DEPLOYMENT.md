# Docker Deployment Guide

This guide covers deploying embed_star using Docker and Docker Compose.

## Quick Start

1. **Clone the repository and navigate to the project directory**
   ```bash
   git clone <repository-url>
   cd embed_star
   ```

2. **Copy environment variables**
   ```bash
   cp .env.example .env
   # Edit .env with your configuration
   ```

3. **Build and start the services**
   ```bash
   docker-compose up -d
   ```

## Available Services

### Core Services (always running)
- **embed_star**: The main embedding service (port 9090)
- **surrealdb**: Database for storing repository data (port 8000)

### Optional Services (use profiles)
- **ollama**: Local embedding provider (profile: `ollama`)
- **prometheus**: Metrics collection (profile: `monitoring`)
- **grafana**: Metrics visualization (profile: `monitoring`)

## Configuration

### Using Different Embedding Providers

1. **Ollama (Local)**
   ```bash
   docker-compose --profile ollama up -d
   ```
   Environment:
   ```env
   EMBEDDING_PROVIDER=ollama
   EMBEDDING_MODEL=nomic-embed-text
   ```

2. **OpenAI**
   ```env
   EMBEDDING_PROVIDER=openai
   EMBEDDING_MODEL=text-embedding-3-small
   OPENAI_API_KEY=your-api-key
   ```

3. **Together AI**
   ```env
   EMBEDDING_PROVIDER=together
   EMBEDDING_MODEL=togethercomputer/m2-bert-80M-8k-retrieval
   TOGETHER_API_KEY=your-api-key
   ```

### Enabling Monitoring

Start with monitoring profile:
```bash
docker-compose --profile monitoring up -d
```

Access:
- Prometheus: http://localhost:9091
- Grafana: http://localhost:3000 (admin/admin)

## Common Operations

### View logs
```bash
docker-compose logs -f embed_star
```

### Restart service
```bash
docker-compose restart embed_star
```

### Scale workers
```bash
# Edit PARALLEL_WORKERS in .env, then:
docker-compose up -d
```

### Check health
```bash
curl http://localhost:9090/health
```

### View metrics
```bash
curl http://localhost:9090/metrics
```

## Production Deployment

### Using Docker Swarm
```bash
docker stack deploy -c docker-compose.yml embed_star
```

### Using Kubernetes
1. Build and push image:
   ```bash
   docker build -t your-registry/embed_star:latest .
   docker push your-registry/embed_star:latest
   ```

2. Apply Kubernetes manifests:
   ```bash
   kubectl apply -f k8s/
   ```

### Environment Variables

Key production settings:
- `RUST_LOG`: Set to `warn,embed_star=info` for production
- `POOL_MAX_SIZE`: Increase for high load (default: 10)
- `PARALLEL_WORKERS`: Number of concurrent batch processors (default: 3)
- `BATCH_SIZE`: Repos per batch (default: 10)

## Troubleshooting

### Service won't start
- Check logs: `docker-compose logs embed_star`
- Verify SurrealDB is healthy: `docker-compose ps`
- Check environment variables in `.env`

### Out of memory
- Reduce `BATCH_SIZE` and `PARALLEL_WORKERS`
- Increase Docker memory limit

### Slow performance
- Enable monitoring to identify bottlenecks
- Adjust `POOL_MAX_SIZE` for database connections
- Use local Ollama for faster embeddings

### Connection errors
- Ensure services are on same network
- Check firewall rules
- Verify service names in configuration