# Build stage
FROM rust:1.75-alpine AS builder

# Install build dependencies
RUN apk add --no-cache musl-dev openssl-dev pkgconfig

# Create app directory
WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Build dependencies - this is cached
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# Copy source code
COPY src ./src

# Build for release
RUN cargo build --release

# Runtime stage
FROM alpine:latest

# Install runtime dependencies
RUN apk add --no-cache ca-certificates libgcc

# Create non-root user
RUN addgroup -g 1000 embed_star && \
    adduser -D -s /bin/sh -u 1000 -G embed_star embed_star

# Copy binary from builder
COPY --from=builder /app/target/release/embed_star /usr/local/bin/embed_star

# Set ownership
RUN chown embed_star:embed_star /usr/local/bin/embed_star

# Switch to non-root user
USER embed_star

# Expose monitoring port
EXPOSE 9090

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD wget --no-verbose --tries=1 --spider http://localhost:9090/health || exit 1

# Run the binary
ENTRYPOINT ["/usr/local/bin/embed_star"]