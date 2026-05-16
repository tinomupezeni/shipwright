# Multi-stage build for Shipwright Agent
# Produces a minimal container image for production deployment

FROM rust:1.83-slim as builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy entire workspace (needed for workspace resolution)
COPY Cargo.toml Cargo.lock ./
COPY common ./common
COPY agent ./agent
COPY cli ./cli

# Build the agent in release mode with locked dependencies
RUN cargo build --release --locked --package shipwright-agent

# Runtime stage - minimal image
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    git \
    openssh-client \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user for security
RUN useradd -m -u 1000 shipwright

# Create required directories
RUN mkdir -p /var/lib/shipwright /etc/shipwright && \
    chown -R shipwright:shipwright /var/lib/shipwright /etc/shipwright

# Copy binary from builder
COPY --from=builder /build/target/release/shipwright-agent /usr/local/bin/shipwright-agent

# Set permissions
RUN chmod +x /usr/local/bin/shipwright-agent

# Switch to non-root user
USER shipwright

# Set working directory
WORKDIR /var/lib/shipwright

# Expose ports
EXPOSE 8081 8084

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8084/health || exit 1

# Set environment variables
ENV RUST_LOG=info

# Run the agent
CMD ["/usr/local/bin/shipwright-agent"]
