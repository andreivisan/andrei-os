# Andrei OS

Andrei OS is one application for managing everything that needs and can be managed in my life.

## Architecture

Version 1 is also a Rust learning project. The backend uses Axum and Tokio, with SQLx and SQLite for persistence.

The project is organized by feature. Application startup and external systems are kept outside the feature modules:

```text
src/
├── main.rs
├── application/
│   └── mod.rs
├── infrastructure/
│   ├── mod.rs
│   └── database/
│       └── mod.rs
└── modules/
    ├── mod.rs
    └── expenses/
        ├── mod.rs
        ├── models.rs
        ├── repository.rs
        ├── handlers.rs
        └── router.rs
```

### Responsibilities

- `main.rs` starts the Tokio runtime and hands control to the application.
- `application` coordinates startup: configuration, infrastructure, routers, and the HTTP server.
- `infrastructure/database` creates and configures the SQLx connection pool and handles database-level setup such as migrations.
- `modules` contains the application's feature modules, such as the expense tracker, capture, fitness, and second brain.
- `router.rs` declares the Axum routes owned by a feature.
- `handlers.rs` handles the HTTP boundary: extracting requests, calling application logic, and producing responses.
- `repository.rs` contains the feature's SQL queries and database mappings.
- `models.rs` contains the feature's domain structs, enums, and their implementations.
- A `service.rs` file is added only when a feature develops business logic that does not belong in an HTTP handler or repository.

Middleware is an HTTP concern and remains separate from handlers and business logic. A transaction spanning multiple repository operations is coordinated by the service or use case that owns the complete operation.

### Dependency flow

```text
main
  → application::run
      → infrastructure::database creates the SQLx pool
      → modules::router receives application state
          → handler
              → service, when needed
                  → repository
                      → SQLx pool
```

Infrastructure owns how the database pool is created, but the pool is not stored in a global variable. It is returned to the application and passed explicitly through Axum state so dependencies remain visible and testable.

## Production readiness

The current architecture is suitable to build on, but the following work should be completed before exposing Andrei OS as a production service.

### Configuration and startup

- Make the bind address configurable, for example with `ANDREI_OS_BIND_ADDRESS`. `127.0.0.1` is appropriate for local-only access; containers and remote clients commonly require `0.0.0.0`, which must not be used without appropriate authentication and network protection.
- Continue providing production configuration through process environment variables. Validate required variables at startup and report the name of any missing or invalid variable without exposing secrets.
- Let startup fail before accepting traffic when configuration, the database connection, or migrations are invalid. Prefer structured startup errors and a non-zero process exit over panics without context.
- Gate Unix-specific signal imports with `#[cfg(unix)]` so the project still compiles on non-Unix targets.
- Test graceful shutdown and ensure in-flight requests are allowed to finish.

### Database and migrations

- Configure SQLite for the expected web workload. Consider WAL journal mode, keep the pool small, and load-test concurrent reads and writes.
- Add a root `build.rs` that prints `cargo:rerun-if-changed=src/infrastructure/database/sqlite/migrations` so newly added embedded migrations trigger recompilation.
- Add `*.sql text eol=lf` to `.gitattributes` so migration checksums are stable across operating systems.
- Treat every applied migration as immutable. Make schema changes with a new migration and test clean installation as well as upgrading an existing database.
- Store the SQLite file on persistent local storage with correct filesystem permissions. Do not place it on an unsuitable shared or ephemeral filesystem.
- Run only one application instance against the SQLite file. Move to a client/server database such as PostgreSQL before introducing multiple application instances.
- Establish automated backups, retention, and a tested restore procedure. A backup is not trusted until restoration has been verified.

### HTTP and security

- Add authentication and authorization before making personal data reachable outside a trusted local machine.
- Terminate TLS either in the application or, more commonly, at a trusted reverse proxy.
- Add request timeouts, request-body size limits, and appropriate concurrency limits.
- Configure CORS only when a frontend on a different origin requires it; do not default to allowing every origin.
- Return consistent JSON error responses and log internal details instead of exposing database or implementation errors to clients.
- Add request tracing and avoid logging secrets, credentials, or sensitive personal data.

### Observability

- Install and initialize `tracing-subscriber` once in the executable so existing `tracing` events are actually recorded.
- Configure log levels through `RUST_LOG`, with sensible production defaults and noisier local overrides.
- Add liveness and readiness endpoints. Readiness should fail when the application cannot serve requests safely, including when its database dependency is unavailable.
- Ensure startup, migration, request failure, and graceful-shutdown events are visible in logs.

### Testing and delivery

- Add repository tests against an isolated SQLite database and API integration tests covering the complete router-to-database flow.
- Test migrations against a fresh database and a copy representing the previously released schema.
- Avoid pooled `sqlite::memory:` tests unless the pool is restricted to one connection; separate in-memory SQLite connections otherwise see separate databases. A temporary database file is usually less surprising.
- Add CI checks for formatting, compilation, Clippy, tests, and dependency vulnerability auditing.
- Produce and test release builds with `cargo build --release`, keep `Cargo.lock` committed, and deploy the exact artifact that passed CI.
- Run the service as a non-root operating-system user with access limited to the application and database paths it requires.
- Reduce broad dependency features such as Tokio's `full` feature to the capabilities actually used once the implementation has stabilized.
