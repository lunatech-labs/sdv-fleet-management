FROM rust:1.93-slim as builder

WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY src ./src

RUN cargo build --release

# ── Runtime ───────────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

COPY --from=builder /app/target/release/backend /usr/local/bin/backend

CMD ["backend"]
