# CLAUDE.md — URL Shortener (Rust, Production)

## Overview

Minimal URL shortener. One binary, one database, no cache layer.

```
Client → Axum → PostgreSQL
```

## Tech Stack

| Crate        | Purpose                    |
|--------------|----------------------------|
| `axum`       | HTTP server                |
| `tokio`      | Async runtime              |
| `sqlx`       | Async PostgreSQL           |
| `serde`      | JSON serialization         |
| `tracing`    | Logs                       |
| `nanoid`     | Short code generation      |
| `tower-http` | Middleware (CORS, tracing) |

## Project Structure

```
src/
├── main.rs      # server bootstrap, router, AppState
├── routes.rs    # all handlers (GET, POST)
└── db.rs        # sqlx queries

migrations/
└── 001_initial.sql
```

## API Methods

### GET /:code
Redirect to the target URL.
- `302 Found` with `Location` header → target URL
- `404 Not Found` if code doesn't exist

### POST /api/links
Create a new short URL.
- `200 OK` with JSON response
- `400 Bad Request` if URL is empty
- `500 Internal Server Error` on database error

**Request:**
```json
{
  "url": "https://example.com/very/long/path"
}
```

**Response:**
```json
{
  "code": "V1StGXR8",
  "short_url": "http://localhost:8080/V1StGXR8"
}
```

## Database Schema

```sql
-- migrations/001_initial.sql
CREATE TABLE urls (
    id         BIGSERIAL    PRIMARY KEY,
    code       TEXT         NOT NULL UNIQUE,
    url        TEXT         NOT NULL,
    created_at TIMESTAMPTZ  NOT NULL DEFAULT now()
);

CREATE INDEX idx_urls_code ON urls(code);
```

## Short Code Generation

Uses `nanoid::nanoid!(5)` — cryptographically random, URL-safe, no sequential enumeration.
Example: `V1StG`

**Collision handling:** Generate code → INSERT → on unique constraint violation, retry up to `MAX_COLLISION_ATTEMPTS` times (configurable via environment variable).

## URL Cache (In-Memory)

GET `/{code}` requests first hit in-memory LRU cache (moka). On cache miss, fetch from PostgreSQL and insert into cache.

**Benefits:**
- Typical URL shortener traffic is 95–99% reads with Zipf-distributed access (few links get most traffic)
- Cache can eliminate 80–95% of database queries
- Reduces latency from ~5–10ms (DB) to ~100µs (cache)

**Trade-offs:**
- 5-minute TTL by default (configurable via `CACHE_TTL_SECS`) means stale reads up to that window
- Acceptable for shorteners: links are immutable after creation
- To invalidate immediately, would need a invalidation endpoint or pub-sub system (future work)

**Configuration:**
- `CACHE_MAX_CAPACITY`: Number of entries (default 100k, ~10MB memory)
- `CACHE_TTL_SECS`: Seconds before expiration (default 300)

## Security

**SSRF Protection:** POST `/api/links` validates the target URL to prevent redirecting to internal services:
- Blocks `localhost`, `*.local`, `*.internal` domains
- Blocks IPv4 loopback, private ranges, link-local, unspecified (0.0.0.0), RFC 6598 shared space (100.64.0.0/10)
- Blocks IPv6 loopback, unicast link-local, unique local addresses
- Blocks IPv4-mapped IPv6 addresses (e.g., `::ffff:127.0.0.1`) by converting and checking as IPv4

**Path Parameter Validation:** GET `/{code}` validates code format (1-32 chars, alphanumeric/underscore/hyphen) before querying database to prevent DoS log noise.

## AppState

```rust
#[derive(Clone)]
pub struct AppState {
    pub db:                        PgPool,
    pub base_url:                  Arc<str>,
    pub max_collision_attempts:    usize,
}
```

## Environment Variables

```
DATABASE_URL              postgres://user:pass@localhost/shortener  (required)
BASE_URL                  http://localhost:8080                     (default)
PORT                      8080                                      (default)
RUST_LOG                  info                                      (default)
DB_MAX_CONNECTIONS        50                                        (default)
MAX_COLLISION_ATTEMPTS    3                                         (default)
CACHE_MAX_CAPACITY        100000                                    (default)
CACHE_TTL_SECS            300                                       (default)
```

**DB_MAX_CONNECTIONS**: PostgreSQL connection pool size. Tune based on concurrency:
- 10-20 concurrent clients: 20-30
- 100+ concurrent clients: 50-100
- High-load (10k+ concurrent): 100-200

Higher values increase memory usage (~10MB per connection). Recommended for load testing: 50+.

**CACHE_MAX_CAPACITY**: In-memory URL cache size (entries). Tune based on working set:
- Small (~1k unique links): 10,000
- Medium (~100k unique links): 100,000
- Large (>1M unique links): 500,000+

Each entry stores code + URL (~100 bytes avg). Cache uses LRU eviction.

**CACHE_TTL_SECS**: URL cache entry lifetime in seconds. Allows expiration of stale mappings:
- Short (60s): Frequent URL updates
- Medium (300s / 5min): Typical case, rare updates
- Long (3600s / 1h): Static links, reduce DB load

## Testing

### Unit Tests
Run validation and logic tests (no database required):
```bash
cargo test --lib
```

Tests cover:
- URL code generation validation
- URL format validation  
- Request/response serialization
- Handler logic

### Integration Tests
Full API endpoint tests (requires PostgreSQL):
```bash
export DATABASE_URL=postgres://user:pass@localhost/shortener
cargo test --test integration_tests -- --ignored
```

## Development

```bash
# Setup
export DATABASE_URL=postgres://user:pass@localhost/shortener
cargo build

# Run
cargo run

# Hot reload
cargo watch -x run

# Test all
cargo test
```

## Build & Deploy

### Local Binary
```bash
cargo build --release
./target/release/ligilo
```

### Docker

**Local Development** (with PostgreSQL):
```bash
docker-compose -f compose.dev.yml up
```
Includes PostgreSQL 18, builds app locally.

**With External Database**:
```bash
export DATABASE_URL="postgres://user:pass@db.example.com/shortener"
export BASE_URL="https://short.example.com"
docker compose up
```
Uses `compose.yml`, pulls pre-built image from GHCR, supports env var overrides.

The binary runs migrations automatically on startup.

### Deployment
Use `compose.yml` with environment variables:

```bash
export DATABASE_URL="postgres://user:pass@your-db.example.com/shortener"
export BASE_URL="https://short.example.com"
export PORT=8080
export RUST_LOG=info

docker compose up -d
```

Supports fallback defaults if env vars not set:
- `DATABASE_URL`: defaults to localhost shortener DB
- `BASE_URL`: defaults to `http://localhost:8080`
- `PORT`: defaults to `8080`
- `RUST_LOG`: defaults to `info`

Use managed PostgreSQL (AWS RDS, Google Cloud SQL, etc.).

## CI/CD

GitHub Actions automatically:
1. Runs tests on every push and PR
2. Publishes multi-platform Docker images to GHCR on push to `master` and on version tags

### Test Workflow (`.github/workflows/test.yml`)
- Unit tests (`cargo test --lib`)
- Integration tests against PostgreSQL service container
- Code formatting check
- Clippy linting
- Release build

Runs on every push and pull request.

### Publish Workflow (`.github/workflows/publish.yml`)
Builds and pushes multi-platform Docker images with:
- **Platforms:** `linux/amd64`, `linux/arm64` (parallel build)
- **Registry:** GitHub Container Registry (GHCR)
- **Tags:**
  - `latest` — on push to `master`
  - `master` — on push to `master`
  - `v1.0.0`, `1.0`, `1` — on version tags (e.g., `v1.0.0`)
  - `sha-abc1234` — commit short SHA

Triggered on:
- Push to `master` branch
- Git tags matching `v*` (e.g., `v1.0.0`)

**Pull images:**
```bash
docker pull ghcr.io/taranovegor/ligilo:latest
docker pull ghcr.io/taranovegor/ligilo:master
docker pull ghcr.io/taranovegor/ligilo:v1.0.0
```

**Docker layer caching:** Enabled for faster incremental builds on subsequent pushes.

**Authentication:** Uses `GITHUB_TOKEN` (automatic in GitHub Actions)