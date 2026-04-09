FROM rust:1.94-trixie AS builder

WORKDIR /app
COPY . .

RUN cargo build --release

FROM debian:trixie-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/ligilo /usr/local/bin/ligilo
COPY migrations /app/migrations

WORKDIR /app

ENV RUST_LOG=info
ENV PORT=8080

EXPOSE 8080

ENTRYPOINT ["ligilo"]
