# Calibration Scope Dashboard — Architecture Plan
## Foundation-Grade Rust Backend + SSE Dashboard

### Mission
Scientific benchmarking dashboard: lightweight, fast, secure, replicable, scalable, beautiful, Apple-friendly on mobile. Built for the future, not for today's convenience.

### Confirmed Stack (from documentation)
- **Rust 1.96.1** (stable, installed)
- **axum 0.8.9** — HTTP routing, SSE first-class via `Sse<impl Stream<Item=Result<Event, Infallible>>>` with `KeepAlive`
- **tokio** — async runtime, concurrent connections (no fork() hack)
- **sqlx 0.9** — async SQLite with compile-time SQL validation via `query!` macro (SQL errors = compiler errors), versioned migrations via `sqlx::migrate!()`
- **serde + serde_json** — type-safe JSON
- **tower-http** — CORS, tracing, static file serving
- **thiserror** — proper error types
- **tracing** — structured logging

### Project Structure (modular, scalable)
```
calibration-scope/
├── Cargo.toml
├── .env                    # DATABASE_URL for sqlx compile-time checking
├── .sqlx/                  # Offline SQL query cache (checked into git)
├── migrations/
│   └── 001_init.sql        # Versioned schema migration
├── src/
│   ├── main.rs             # Entry point, server bootstrap
│   ├── config.rs           # Configuration (env vars, paths)
│   ├── error.rs            # Error types (thiserror)
│   ├── state.rs            # AppState (DB pool, config)
│   ├── routes/
│   │   ├── mod.rs          # Router assembly
│   │   ├── index.rs        # GET / — serve dashboard.html
│   │   ├── status.rs       # GET /api/status
│   │   ├── summary.rs      # GET /api/summary
│   │   ├── models.rs       # GET /api/models
│   │   └── events.rs       # GET /api/events (SSE)
│   ├── models/
│   │   ├── mod.rs
│   │   ├── benchmark.rs     # BenchmarkRow struct
│   │   └── model_entry.rs   # ModelEntry struct
│   └── db/
│       ├── mod.rs          # Database connection pool setup
│       └── queries.rs       # SQL queries (compile-time checked)
├── assets/
│   ├── dashboard.html      # SSE client, responsive, Apple-friendly
│   └── owl.png             # Branding
└── data/
    └── calibration_scope.sqlite  # SQLite database
```

### API Endpoints
| Method | Path | Description |
|--------|------|-------------|
| GET | `/` | Serve dashboard.html |
| GET | `/assets/owl.png` | Serve owl branding image |
| GET | `/api/status` | Health check → `ok` |
| GET | `/api/summary` | All benchmark rows as JSON array |
| GET | `/api/models` | Unique models as JSON array (deduplicated) |
| GET | `/api/events` | SSE stream — pushes data on DB changes |

### SSE Architecture
- Server holds connection open via `Sse<impl Stream>`
- `KeepAlive` sends heartbeat every 5s to detect disconnected clients
- Stream queries DB every 2s, compares content hash, pushes `data:` event only on change
- tokio handles concurrency: multiple SSE clients + API requests simultaneously
- Auto-reconnect: frontend `EventSource` reconnects automatically on disconnect (3s retry)

### Database Layer
- SQLite now (read-only connection pool via `SqlitePool`)
- `DATABASE_URL` env var for compile-time SQL validation
- Offline mode: `.sqlx` directory cached for reproducible builds without DB connection
- PostgreSQL migration path: swap connection string in `.env`, run `sqlx::migrate!()` — zero code changes
- Versioned migrations in `migrations/` directory

### Error Handling
- `thiserror` for typed application errors
- All handlers return `Result<Response, AppError>`
- Structured error responses with proper HTTP status codes
- `tracing::error!` for server-side logging

### Frontend Requirements
- SSE via `EventSource` — no polling, no refresh button
- Responsive: works on mobile (viewport meta, flex-wrap, touch-friendly controls)
- Dark theme matching macOS aesthetic (`color-scheme: dark`)
- Apple-friendly: `-apple-system` font, `BlinkMacSystemFont`
- Color-coded stat cards (green/red/amber)
- Connection status indicator (Live/Disconnected/Reconnecting)
- Provider tags with color coding
- Detail text wrapping (no truncation)
- Auto-reconnect on SSE disconnect

### launchd Service
- Point at `cargo build --release` binary
- `KeepAlive` + `RunAtLoad`
- Survives reboots
- Log to `~/.hermes/logs/`

### Testing
- Integration tests: spawn server, test SSE stream, test API endpoints
- `cargo test` with `#[tokio::test]`
- Verify real data flows through the entire pipeline

### Build & Deploy
- `cargo build --release` — optimized binary
- `cargo sqlx prepare` — cache SQL queries for offline compilation
- `cargo test` — run integration tests
- Binary path: `target/release/calibration-scope-dashboard`
