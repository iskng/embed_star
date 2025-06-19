# SurrealDB Connection Types

The embed_star service supports multiple SurrealDB connection types based on the URL scheme you provide:

## Supported URL Schemes

### WebSocket Connections (Recommended)
```bash
# Standard WebSocket (unencrypted)
DB_URL=ws://localhost:8000

# Secure WebSocket (TLS/SSL)
DB_URL=wss://your-surrealdb-server.com:8000
```

### HTTP Connections
```bash
# Standard HTTP (will be converted to ws://)
DB_URL=http://localhost:8000

# HTTPS (will be converted to wss://)
DB_URL=https://your-surrealdb-server.com:8000
```

## How It Works

The connection type is automatically determined from your DB_URL:
- `ws://` → WebSocket connection
- `wss://` → Secure WebSocket connection
- `http://` → Converted to `ws://` for compatibility
- `https://` → Converted to `wss://` for compatibility

## Examples

### Local Development
```bash
# Using standard WebSocket
export DB_URL=ws://localhost:8000
```

### Production with TLS
```bash
# Using secure WebSocket
export DB_URL=wss://db.production.com:8000
```

### Cloud Services
```bash
# If your provider gives you an HTTP endpoint
export DB_URL=https://your-instance.surrealdb.cloud:8000
# This will automatically use wss:// for the connection
```

## Troubleshooting

### Connection Timeout
If you see "Operation timed out", check:
1. SurrealDB is running and accessible
2. The URL and port are correct
3. Firewall/security groups allow the connection
4. For cloud services, verify the endpoint is active

### Invalid URL Scheme
If you see "Unsupported URL scheme", ensure your DB_URL starts with one of:
- `ws://`
- `wss://`
- `http://`
- `https://`

### TLS/SSL Issues
For `wss://` or `https://` connections:
- Ensure valid SSL certificates
- Check certificate chain is complete
- Verify hostname matches certificate

## Performance Considerations

- WebSocket connections (`ws://`, `wss://`) are preferred for real-time updates
- HTTP connections are converted to WebSocket for compatibility
- For production, always use secure connections (`wss://` or `https://`)