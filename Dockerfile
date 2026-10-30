# ==============================================================================
# AgentCore Standalone Multi-stage Dockerfile
# ==============================================================================
FROM rust:1.80-slim AS builder
WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml ./
COPY crates ./crates

# Build release binaries for both CLI and Server
RUN cargo build --release -p agent-cli -p agent-server

# Stage 2: Minimal runtime image
FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/agent-ctl /usr/local/bin/agent-ctl
COPY --from=builder /app/target/release/agent-server /usr/local/bin/agent-server

EXPOSE 8080

CMD ["agent-server", "--host", "0.0.0.0", "--port", "8080", "--workspace", "/app/data"]
