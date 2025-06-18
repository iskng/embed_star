# Production Readiness Checklist

This checklist helps ensure the embed_star service is ready for production deployment.

## Prerequisites ✅

- [ ] **SurrealDB** v1.5+ running and accessible
- [ ] **Embedding Provider** configured:
  - [ ] Ollama (local) with model installed
  - [ ] OR OpenAI API key
  - [ ] OR Together AI API key
- [ ] **Environment Variables** configured (see `.env.example`)
- [ ] **Database Schema** migrations applied

## Pre-Deployment Testing

### 1. Local Testing
```bash
# Run unit tests
cargo test

# Run integration tests (requires SurrealDB + Ollama)
cargo test -- --ignored

# Run the example
cargo run --example production_run

# Test with production config
cargo run --release -- --batch-size 50 --parallel-workers 4
```

### 2. Load Testing
```bash
# Create 1000 test repositories
# Monitor:
# - Memory usage stays under 500MB
# - CPU usage scales with parallel workers
# - Embeddings generated at expected rate
# - No memory leaks over time
```

### 3. Error Handling Testing
- [ ] Test with embedding provider down
- [ ] Test with database connection loss
- [ ] Test with rate limit exceeded
- [ ] Test graceful shutdown during processing

## Configuration Review

### Required Environment Variables
```bash
# Database
DB_URL=ws://your-surrealdb:8000
DB_USER=your-user
DB_PASS=your-password
DB_NAMESPACE=gitstars
DB_DATABASE=stars

# Embedding Provider (choose one)
EMBEDDING_PROVIDER=ollama
OLLAMA_URL=http://localhost:11434
EMBEDDING_MODEL=nomic-embed-text

# Performance Tuning
BATCH_SIZE=20              # Adjust based on provider limits
PARALLEL_WORKERS=3         # Based on CPU cores
BATCH_DELAY_MS=100        # Prevent overwhelming providers
POOL_SIZE=10              # Database connections

# Monitoring
MONITORING_PORT=9090      # Prometheus metrics
RUST_LOG=warn,embed_star=info
```

### Security Checklist
- [ ] API keys stored securely (not in code)
- [ ] Database credentials encrypted
- [ ] Network policies configured
- [ ] TLS enabled for external connections
- [ ] Service runs as non-root user

## Deployment Steps

### 1. Docker Deployment
```bash
# Build image
docker build -t embed_star:latest .

# Run with production config
docker run -d \
  --name embed_star \
  -p 9090:9090 \
  --env-file .env.production \
  --restart unless-stopped \
  embed_star:latest
```

### 2. Kubernetes Deployment
```bash
# Create secrets
kubectl create secret generic embed-star-secrets \
  --from-env-file=.env.production

# Apply manifests
kubectl apply -f deployments/kubernetes/

# Verify deployment
kubectl get pods -l app=embed-star
kubectl logs -f deployment/embed-star
```

### 3. Health Checks
```bash
# Liveness probe
curl http://localhost:9090/livez

# Readiness probe  
curl http://localhost:9090/health

# Metrics
curl http://localhost:9090/metrics | grep embed_star
```

## Monitoring Setup

### 1. Key Metrics to Monitor
- `embed_star_embeddings_total` - Rate of embedding generation
- `embed_star_embeddings_errors_total` - Error rate by type
- `embed_star_repos_pending` - Backlog size
- `embed_star_circuit_breaker_state` - Service availability
- `embed_star_embedding_duration_seconds` - Performance

### 2. Alerts to Configure
```yaml
# High error rate
- alert: HighEmbeddingErrorRate
  expr: rate(embed_star_embeddings_errors_total[5m]) > 0.1
  
# Large backlog
- alert: LargePendingBacklog
  expr: embed_star_repos_pending > 1000
  
# Circuit breaker open
- alert: CircuitBreakerOpen
  expr: embed_star_circuit_breaker_state == 1
```

### 3. Dashboards
- Import provided Grafana dashboard from `deployments/grafana/`
- Monitor:
  - Embedding rate (per minute)
  - Error rate by provider
  - Cache hit rate
  - Worker utilization
  - Database connection health

## Performance Validation

### Expected Performance
With default configuration:
- **Throughput**: 30-50 repos/second (fresh), 100+ repos/second (cached)
- **Latency**: <100ms (cached), 200-500ms (fresh)
- **Memory**: ~200-400MB steady state
- **CPU**: Scales linearly with workers

### Bottleneck Identification
1. Check metrics for rate limiting
2. Monitor database query performance
3. Review circuit breaker states
4. Analyze worker utilization

## Rollback Plan

1. Keep previous version tagged and ready
2. Database migrations are forward-compatible
3. Can run multiple versions simultaneously (with deduplication)
4. Graceful shutdown ensures no data loss

## Post-Deployment Verification

- [ ] All workers started successfully
- [ ] Metrics endpoint accessible
- [ ] Embeddings being generated
- [ ] No errors in logs
- [ ] Memory usage stable
- [ ] Performance meets expectations

## Maintenance Tasks

### Daily
- Monitor error rates and alerts
- Check pending backlog size

### Weekly  
- Review performance metrics
- Check for failed embeddings
- Verify cache effectiveness

### Monthly
- Analyze embedding quality
- Review resource utilization
- Update embedding models if needed

## Troubleshooting

### Common Issues

1. **No embeddings generated**
   - Check embedding provider connectivity
   - Verify API keys/credentials
   - Check circuit breaker state
   - Review rate limits

2. **High memory usage**
   - Reduce cache size
   - Lower batch size
   - Check for memory leaks in metrics

3. **Slow processing**
   - Increase parallel workers
   - Check database performance
   - Review rate limiting
   - Enable debug logging

### Debug Commands
```bash
# Check service health
curl localhost:9090/health | jq

# View current metrics
curl -s localhost:9090/metrics | grep -E "(pending|total|errors)"

# Enable debug logging
RUST_LOG=debug,embed_star=trace cargo run

# Test specific repository
echo "SELECT * FROM repo WHERE full_name = 'owner/repo'" | surreal sql
```

## Sign-off

- [ ] Load tested with production-like data
- [ ] Error scenarios tested and handled
- [ ] Monitoring and alerts configured
- [ ] Documentation updated
- [ ] Team trained on operations
- [ ] Rollback plan tested

**Ready for Production**: ⬜ Yes / ⬜ No

**Approved by**: _______________
**Date**: _______________