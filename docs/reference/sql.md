# SQL Interface Reference

Reference for the `omnia:sql` interface (version `0.1.0`): the operations, the placeholder rule, the portable dialect, and how each `value` case binds and reads back on SQLite and Postgres. The walk-through is in [SQL](../guides/sql.md), and the authoritative WIT is [`crates/wasi-sql/wit/sql.wit`](../../crates/wasi-sql/wit/sql.wit).

- [Operations](#operations)
- [Placeholders](#placeholders)
- [The portable dialect](#the-portable-dialect)
- [Values and column types](#values-and-column-types)
- [Errors](#errors)
- [The `wasi:sql` draft](#the-wasisql-draft)

## Operations

```wit
query: async func(db: string, sql: string, params: list<value>) -> result<list<row>, error>;
execute: async func(db: string, sql: string, params: list<value>) -> result<u64, error>;
batch: async func(db: string, statements: list<statement>) -> result<list<u64>, error>;
```

| Operation | Semantics |
| --------- | --------- |
| `query` | Run `sql` with `params` bound, and return the rows it selects. |
| `execute` | Run `sql` with `params` bound, and return the number of rows it changed. |
| `batch` | Run `statements` in order, in one transaction, and return each one's changed-row count. |

A failing statement rolls back the whole batch, and the call fails with that statement's error.

`db` names a database the deployment declares, and a name it does not declare fails with `not-found`. Guests call these through the `omnia_sdk::SqlDb` trait, one call per method. A provider's own handle passes `default`, and `named(name)` passes `name`.

## Placeholders

A statement numbers its parameters `$1`, `$2`, and so on. `$n` binds the `n`th value of `params` wherever it appears, in any order and as often as it appears. So `UPDATE items SET name = $2 WHERE id = $1` and `WHERE a = $1 OR b = $1` run alike on every backend. Other placeholder styles, such as `?`, are not portable.

## The portable dialect

The host passes any SQL through to the database. SQL outside the portable dialect still runs, but ties the guest to the database it was written for. The portable dialect is:

- The column types are Postgres names that SQLite accepts by affinity: `INTEGER`, `BIGINT`, `REAL`, `DOUBLE PRECISION`, `TEXT`, `BOOLEAN`, `BYTEA`, `DATE`, `TIME`, `TIMESTAMP`, and `TIMESTAMPTZ`.
- The caller supplies ids. A key the database generates, such as `SERIAL` or `AUTOINCREMENT`, is not portable.
- A statement with a `RETURNING` clause runs through `query`, which returns the rows the clause produces.
- `INSERT ... ON CONFLICT (...) DO NOTHING` and `DO UPDATE` run on both.

## Values and column types

Each portable `value` case binds to one column type. `Row::get` reads it back as the Rust type listed, whichever backend returned it.

- `int32` binds to `INTEGER`. Postgres returns `int32` and SQLite `int64`, so read it as `i32`.
- `int64` binds to `BIGINT`. Both return `int64`, which reads as `i64`.
- `float` binds to `REAL`. Postgres returns `float` and SQLite `double`, so read it as `f64`.
- `double` binds to `DOUBLE PRECISION`. Both return `double`, which reads as `f64`.
- `boolean` binds to `BOOLEAN`. Postgres returns `boolean` and SQLite `int64` `0` or `1`, and both read as `bool`.
- `str` binds to `TEXT`. Both return `str`, which reads as `String`.
- `binary` binds to `BYTEA`. Both return `binary`, which reads as `Vec<u8>` or `Bytes`.
- `date` binds to `DATE` as `YYYY-MM-DD`. Postgres returns `date` and SQLite `str`, both as bound, and both read as `String`.
- `time` binds to `TIME` as `HH:MM:SS`. Postgres returns `time` and SQLite `str`, both as bound, and both read as `String`.
- `timestamp` binds to `TIMESTAMP` as `YYYY-MM-DD HH:MM:SS`, or to `TIMESTAMPTZ` as RFC 3339 with a `+00:00` offset, such as `2024-01-02T03:04:05+00:00`. Postgres returns `timestamp` and SQLite `str`, both as bound, and both read as `String`.

Two cases are not portable:

- Postgres binds `uint32` only to its `oid` type.
- Postgres rejects a `uint64` above `i64::MAX`, and SQLite stores one as its raw bits.

A `NULL` binds as its column's case, such as `int64(none)` for a `BIGINT` column. Postgres returns it in that case and SQLite as `str(none)`, and both read as `None` through an `Option<T>`.

## Errors

| Case | Meaning |
| ---- | ------- |
| `not-found` | The deployment does not declare the database. |
| `denied` | The caller may not use the database. |
| `invalid` | The statement was rejected, such as SQL that does not prepare or parameters that do not bind. |
| `unsupported` | A valid request this backend cannot serve. |
| `unavailable` | The database did not answer, or the call exceeded its bound. |
| `other` | Any other failure. |

`omnia_sdk::capability::ErrorKind` has one kind per case, and the SDK maps each case to its kind.

## The `wasi:sql` draft

The host also serves the `wasi:sql` draft for guests that call the raw bindings: `Connection::open`, `Statement::prepare`, and `readwrite::{query, exec}`. It runs over the same backend as `omnia:sql`, with these differences:

- It has no `batch`.
- Each row carries an `index`, its position in the result.
- `exec` counts in a `u32`, so a larger count reads as `u32::MAX`.
- A failure is an `error` resource whose `trace` is the message alone, without its case.
