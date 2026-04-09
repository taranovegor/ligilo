# ligilo!
yet another link shortening service 🚀

[![Test](https://github.com/taranovegor/ligilo/actions/workflows/test.yml/badge.svg)](https://github.com/taranovegor/ligilo/actions)
[![License](https://img.shields.io/badge/license-MIT-green)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.70+-orange)](https://www.rust-lang.org)
[![Docker](https://img.shields.io/badge/docker-ghcr.io-blue)](https://github.com/taranovegor/ligilo/pkgs/container/ligilo)

## Quick Start

### Requirements
- Rust 1.70+
- PostgreSQL 12+

### Option 1: Docker with Local Database (Dev)
```bash
docker-compose -f compose.dev.yml up
```
Includes PostgreSQL + app with local build:
- PostgreSQL: `localhost:5432`
- App: `localhost:8080`

### Option 2: Docker with External Database
```bash
# Set your database URL
export DATABASE_URL="postgres://user:pass@your-db.example.com/shortener"
export BASE_URL="https://short.example.com"
export DB_MAX_CONNECTIONS=50  # tune for your load

docker compose up
```
Uses pre-built image from GHCR (multi-platform: amd64, arm64).
Falls back to defaults if env vars not set.

**Performance Tuning:**
- `DB_MAX_CONNECTIONS=50` for typical workloads
- `DB_MAX_CONNECTIONS=100+` for high concurrency (10k+ concurrent requests)

### Option 3: Local Binary Development
```bash
export DATABASE_URL="postgres://postgres:@localhost/shortener"
export BASE_URL="http://localhost:8080"

cargo build --release
./target/release/ligilo
```

The server automatically runs migrations on startup.

## API Documentation

Full OpenAPI specification available at: **https://taranovegor.github.io/ligilo/**

Interactive API documentation with endpoint details, request/response examples, and SSRF protection details.

## API

### Create a short URL
```bash
curl -X POST http://localhost:8080/api/links \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com/very/long/path"}'
```

Response:
```json
{
  "code": "V1StGXR8",
  "short_url": "http://localhost:8080/V1StGXR8"
}
```

### Redirect to original URL
```bash
curl -L http://localhost:8080/V1StGXR8
```

## Architecture

See [CLAUDE.md](CLAUDE.md) for detailed technical documentation.

## License

MIT License — see [LICENSE](LICENSE) for details.
