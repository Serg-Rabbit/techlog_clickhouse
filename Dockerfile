FROM rust:1-slim AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY sql ./sql
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim

WORKDIR /app
COPY --from=builder /app/target/release/techlog-clickhouse /usr/local/bin/techlog-clickhouse
ENTRYPOINT ["techlog-clickhouse"]
