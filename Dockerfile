# ── Stage 1: Build ───────────────────────────────────────────────────────────
FROM rust:1.85-bookworm AS builder

# Install build dependencies for OpenSSL vendored build and other native libs
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev cmake perl make gcc g++ \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy manifests first for layer caching
COPY Cargo.toml Cargo.lock ./
COPY crates/rustyclaw-core/Cargo.toml crates/rustyclaw-core/Cargo.toml
COPY crates/rustyclaw-cli/Cargo.toml  crates/rustyclaw-cli/Cargo.toml
COPY crates/rustyclaw-tui/Cargo.toml  crates/rustyclaw-tui/Cargo.toml

# Create stub lib.rs / main.rs so cargo can resolve the workspace
RUN mkdir -p crates/rustyclaw-core/src && echo "" > crates/rustyclaw-core/src/lib.rs \
    && mkdir -p crates/rustyclaw-cli/src  && echo "fn main(){}" > crates/rustyclaw-cli/src/main.rs \
    && mkdir -p crates/rustyclaw-tui/src  && echo "fn main(){}" > crates/rustyclaw-tui/src/main.rs

# Pre-build dependencies (cached unless Cargo.toml/lock change)
RUN cargo build --release --workspace 2>/dev/null || true

# Copy actual source
COPY . .

# Touch source files so cargo re-compiles them (not the deps)
RUN find crates -name "*.rs" -exec touch {} +

# Build the CLI binary (the main entry point)
RUN cargo build --release --bin rustyclaw

# ── Stage 2: Runtime ─────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    poppler-utils \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd -r rustyclaw && useradd -r -g rustyclaw -m rustyclaw

# Copy binary
COPY --from=builder /build/target/release/rustyclaw /usr/local/bin/rustyclaw

# Default config location
RUN mkdir -p /home/rustyclaw/.config/rustyclaw && \
    chown -R rustyclaw:rustyclaw /home/rustyclaw

USER rustyclaw
WORKDIR /home/rustyclaw

# Gateway port
EXPOSE 3000

# Health check using the /health endpoint
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -sf http://localhost:3000/health || exit 1

ENTRYPOINT ["rustyclaw"]
CMD ["--help"]
