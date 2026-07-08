# Stage 1: Build
FROM rust:1-bookworm AS builder

# tree-sitter + tiktoken-rs need a C toolchain; openssl for HTTPS/git
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    build-essential \
    cmake \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/

# CLI only (no GUI) — matches release.yml artifact build
RUN cargo build --release --locked -p arbor-graph-cli

# Stage 2: Runtime
FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
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
