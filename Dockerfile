# Stage 1: Build
FROM rust:1.85-slim AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/

RUN cargo build --release --manifest-path crates/arbor-cli/Cargo.toml --bin arbor

# Stage 2: Runtime
FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
    git \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/arbor /usr/local/bin/arbor

LABEL org.opencontainers.image.source="https://github.com/Anandb71/arbor"
LABEL org.opencontainers.image.description="Arbor: Graph-native intelligence for codebases"
LABEL org.opencontainers.image.licenses="MIT"

# MCP servers communicate via stdio
ENTRYPOINT ["arbor", "bridge"]
