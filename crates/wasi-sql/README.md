# Omnia WASI SQL

This crate provides the SQL database interface for the Omnia runtime.

## Interface

Implements `omnia:sql`, the interface the SDK's `SqlDb` calls, and the `wasi:sql` draft for raw-binding guests. Both are served from one `Connection` per backend.

A guest reaches `omnia:sql` through `omnia_sdk::SqlDb`, one host call per method. The provider's own handle runs statements on the database `default`, and `named(name)` on another database the host declares.

- A name the deployment does not declare fails with `not-found`.
- `batch` runs its statements in one transaction, and a failing statement rolls back the whole batch.
- The [SQL reference](https://github.com/augentic/omnia/blob/main/docs/reference/sql.md) lists the portable dialect, and how each value binds and reads back on each backend.

## Backend

- **Host**: Uses `rusqlite` to provide a `SQLite` backend. Supports both in-memory (`:memory:`) and file-based databases.

## Configuration

`SqlDefault` holds one `SQLite` database per declared name and refuses to open any other. Names match case-insensitively, and dates, times, and timestamps are stored as the text they are given.

| Variable | Purpose |
| -------- | ------- |
| `SQL_DATABASE` | The URI of the database `default`. Unset, it is one in-memory database shared across the process. |
| `SQL_DATABASES` | The further names a guest may open, comma-separated. |
| `SQL_DATABASE__<NAME>` | The URI of the listed name `<name>`. Unset, the name gets a private in-memory database. |

## Cargo features

- `sqlite` *(default)*: the bundled-SQLite default backend (`SqlDefault`). Disable it (`default-features = false`) when supplying your own `SqlClient` backend to skip compiling bundled `SQLite`.

## Features

- **Production**: [`omnia-postgres`](https://github.com/augentic/omnia-backends/tree/main/crates/postgres) (`PostgreSQL`) — a one-line swap in the host, guests untouched (see the [Production Backends guide](https://github.com/augentic/omnia/blob/main/docs/guides/production-backends.md)).

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia_wasi_sql::{SqlDefault, WasiSql};

omnia::runtime!({
    hosts: {
        WasiSql: SqlDefault,
    }
});
```

## License

MIT OR Apache-2.0
