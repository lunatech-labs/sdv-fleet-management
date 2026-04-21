FROM rust:1.93-slim as builder

RUN apt-get update && apt-get install -y protobuf-compiler && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY build.rs ./
COPY proto ./proto
COPY src ./src

RUN cargo build --release

# ── Runtime ───────────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

COPY --from=builder /app/target/release/ota-agent /usr/local/bin/ota-agent

CMD ["ota-agent"]
