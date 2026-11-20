# Omnia WASI SQL

This crate provides the SQL database interface for the Omnia runtime.

## Interface

Implements `omnia:sql`, the interface the SDK's `sql` module binds, over one
`Connection` per backend.

- A guest opens a database by name through `store`, and runs statements on
  the `database` handle `open` returns. A name the deployment does not
  declare fails with `not-found`.
- `transactions` is optional. A backend that serves none fails `begin` with
  `unsupported`.
- A statement is one SQL statement, and numbers its parameters `$1`, `$2`,
  and so on. `none` binds `NULL`.
- Each `database` call commits before it returns, and `batch` runs its
  statements in one transaction. The host refuses, with `invalid`, a
  statement whose first keyword starts or ends a transaction or a
  savepoint:
  - `BEGIN`, `START`, `COMMIT`, and `END`;
  - `ROLLBACK` and `ABORT`;
  - `SAVEPOINT` and `RELEASE`;
  - `PREPARE TRANSACTION`.
- `SET TRANSACTION` passes, so a transaction's first statement may set its
  isolation level.
- A transaction runs its calls one at a time, in the order made. `commit`
  and `rollback` end it. A failed call rolls it back and ends it, and so
  does dropping it.
- A transaction holds one of the backend's connections until it ends. A
  `database` handle holds the connection `open` returned until its first
  call, and every later call opens one for that call alone.
- A temporal value must be in its case's format, or the call fails with
  `invalid`, naming the parameter:
  - a `date` is `YYYY-MM-DD`;
  - a `time` is `HH:MM:SS`, with optional fractional seconds;
  - a `timestamp` is `YYYY-MM-DD HH:MM:SS`, with optional fractional
    seconds;
  - a `timestamp-tz` is RFC 3339.
- The [SQL reference](https://github.com/augentic/omnia/blob/main/docs/reference/sql.md)
  lists the portable dialect, and how each value binds and reads back on
  each backend.

## Backend

- **Default**: `SqlDefault`, over `rusqlite`'s bundled `SQLite`, in memory
  (`:memory:`) or on file. Every handle on a database shares one
  connection. While a transaction is open, a call on any other handle waits
  for it to end, and fails with `unavailable` after five seconds.

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
