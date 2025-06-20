# Build stage
FROM rust:1.79-slim as builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Create app directory
WORKDIR /app

# Copy manifest files
COPY Cargo.toml Cargo.lock ./

# Build dependencies (this is cached as long as Cargo.toml/lock don't change)
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# Copy source code
COPY src ./src

# Build the application
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 embed_star

# Copy the binary from builder
COPY --from=builder /app/target/release/embed_star /usr/local/bin/embed_star

# Create data directory
RUN mkdir -p /data && chown embed_star:embed_star /data

# Switch to non-root user
USER embed_star

# Set working directory
WORKDIR /data

# Expose monitoring port
EXPOSE 9090

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=40s --retries=3 \
    CMD wget --no-verbose --tries=1 --spider http://localhost:9090/health || exit 1

# Set default environment variables
ENV RUST_LOG=info,embed_star=info \
    DB_URL=ws://surrealdb:8000 \
    DB_NAMESPACE=gitstars \
    DB_DATABASE=stars \
    MONITORING_PORT=9090

# Run the application
ENTRYPOINT ["/usr/local/bin/embed_star"]