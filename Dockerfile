# Stage 1: Build
FROM rust:1.75-slim as builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY crates/ ./crates/

WORKDIR /app/crates
RUN cargo build --release --bin arbor-cli

# Stage 2: Runtime
FROM debian:bookworm-slim
WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/crates/target/release/arbor /usr/local/bin/arbor

# MCP servers communicate via stdio
ENTRYPOINT ["arbor", "bridge"]
