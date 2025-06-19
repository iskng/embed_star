# Connection Pooling Documentation

## Overview

The embed_star service now implements proper connection pooling using deadpool, providing significant performance and reliability improvements over single-connection architectures.

## Benefits

### 1. **Performance**
- **Connection Reuse**: Eliminates overhead of creating new connections for each operation
- **Concurrent Operations**: Multiple database operations can run simultaneously
- **Reduced Latency**: Pre-warmed connections are ready to use immediately

### 2. **Reliability**
- **Health Checks**: Connections are validated before use with periodic health queries
- **Automatic Recovery**: Failed connections are automatically removed and replaced
- **Connection Limits**: Prevents exhausting database resources

### 3. **Scalability**
- **Configurable Size**: Adjust pool size based on workload
- **Dynamic Scaling**: Pool grows as needed up to max_size
- **Resource Management**: Proper connection lifecycle management

## Architecture

```
┌─────────────────────┐
│   Application       │
│  (embed_star)       │
└──────────┬──────────┘
           │
┌──────────▼──────────┐
│   Connection Pool   │
│    (deadpool)       │
│  ┌────┬────┬────┐  │
│  │ C1 │ C2 │ C3 │  │  <- Pooled Connections
│  └────┴────┴────┘  │
└──────────┬──────────┘
           │
┌──────────▼──────────┐
│     SurrealDB       │
└─────────────────────┘
```

## Configuration

### Environment Variables

```bash
# Maximum connections in the pool
POOL_MAX_SIZE=10

# Timeout waiting for an available connection
POOL_WAIT_TIMEOUT_SECS=10

# Timeout for creating new connections
POOL_CREATE_TIMEOUT_SECS=30

# Timeout for recycling connections
POOL_RECYCLE_TIMEOUT_SECS=30
```

### Recommended Settings

#### Development
```bash
POOL_MAX_SIZE=5
POOL_WAIT_TIMEOUT_SECS=5
```

#### Production
```bash
POOL_MAX_SIZE=20
POOL_WAIT_TIMEOUT_SECS=10
```

#### High Load
```bash
POOL_MAX_SIZE=50
POOL_WAIT_TIMEOUT_SECS=30
```

## Implementation Details

### Connection Manager

The `SurrealDBManager` implements deadpool's `Manager` trait:

```rust
pub struct SurrealDBManager {
    config: Arc<Config>,
}

impl Manager for SurrealDBManager {
    type Type = Surreal<Any>;
    type Error = surrealdb::Error;

    async fn create(&self) -> Result<Self::Type, Self::Error> {
        // Creates new connection with authentication
    }

    async fn recycle(&self, conn: &mut Self::Type, _: &Metrics) -> RecycleResult<Self::Error> {
        // Validates connection health before reuse
    }
}
```

### Connection Lifecycle

1. **Creation**: New connections are created with authentication and database selection
2. **Validation**: Health check performed before returning to pool
3. **Recycling**: Connections validated before reuse
4. **Timeout**: Stale connections automatically removed

### Health Checks

Connections are validated using a simple query:
```sql
SELECT 1 as health_check
```

Failed health checks result in connection disposal and replacement.

## Monitoring

### Pool Metrics

The service exposes the following pool metrics:

- `embed_star_pool_connections_active`: Currently active connections
- `embed_star_pool_connections_idle`: Available connections in pool
- `embed_star_pool_connections_waiting`: Requests waiting for connection
- `embed_star_pool_connections_total`: Total connections created
- `embed_star_pool_connection_errors_total`: Connection acquisition failures
- `embed_star_pool_health_check_failures_total`: Failed health checks

### Pool Statistics

Access pool statistics programmatically:

```rust
let stats = pool.stats();
println!("Active: {}, Idle: {}, Waiting: {}", 
    stats.size - stats.available, 
    stats.available, 
    stats.waiting
);
```

## Best Practices

### 1. **Size Pool Appropriately**
- Set `POOL_MAX_SIZE` based on expected concurrent operations
- Monitor `embed_star_pool_connections_waiting` metric
- Increase pool size if consistently seeing waits

### 2. **Monitor Pool Health**
- Watch `embed_star_pool_connection_errors_total`
- Check `embed_star_pool_health_check_failures_total`
- Set alerts for connection failures

### 3. **Handle Connection Errors**
```rust
match pool.get().await {
    Ok(conn) => {
        // Use connection
    }
    Err(e) => {
        // Log error and implement retry logic
        error!("Failed to get connection: {}", e);
    }
}
```

### 4. **Tune Timeouts**
- Adjust timeouts based on network latency
- Longer timeouts for cloud databases
- Shorter timeouts for local databases

## Performance Impact

### Before (Single Connection)
- Sequential operations only
- Connection overhead for each service start
- No failover capability
- Resource contention on single connection

### After (Connection Pool)
- Parallel operations supported
- Amortized connection overhead
- Automatic failover and recovery
- Efficient resource utilization

## Troubleshooting

### Common Issues

1. **"Pool timeout" errors**
   - Increase `POOL_MAX_SIZE`
   - Check for long-running queries
   - Monitor database performance

2. **High connection churn**
   - Check health check query performance
   - Increase `POOL_RECYCLE_TIMEOUT_SECS`
   - Verify network stability

3. **Memory usage**
   - Each connection maintains buffers
   - Reduce `POOL_MAX_SIZE` if memory constrained
   - Monitor connection memory usage

### Debug Logging

Enable debug logs to see pool operations:
```bash
RUST_LOG=embed_star::pool=debug cargo run
```

This will show:
- Connection creation/destruction
- Health check results
- Pool statistics
- Connection acquisition timing