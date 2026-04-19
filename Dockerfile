# Multi-stage build for knot codebase indexer
# Provides universal Linux compatibility (glibc 2.35+)

# Build stage
FROM rust:1.90-slim-trixie AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Set working directory
WORKDIR /build

# Copy source code
COPY . .

# Build release binaries
RUN cargo build --release

# Runtime stage
FROM debian:trixie-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy binaries from builder
COPY --from=builder /build/target/release/knot-indexer /usr/local/bin/knot-indexer
COPY --from=builder /build/target/release/knot-mcp /usr/local/bin/knot-mcp
COPY --from=builder /build/target/release/knot /usr/local/bin/knot

# Create workspace directory
RUN mkdir -p /workspace
WORKDIR /workspace

# Set environment variables with defaults
ENV KNOT_QDRANT_URL=http://localhost:6334
ENV KNOT_NEO4J_URI=bolt://localhost:7687
ENV KNOT_NEO4J_USER=neo4j

# Default command (can be overridden)
CMD ["knot-indexer"]
