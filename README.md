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
