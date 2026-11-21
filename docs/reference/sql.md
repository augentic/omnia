# SQL Interface Reference

Reference for the `omnia:sql` package (version `0.1.0`): its interfaces, the statement rules, transactions, the portable dialect and limits, how each `value` case binds and reads back on SQLite and Postgres, the errors, and the SDK's value mapping. The walk-through is in [SQL](../guides/sql.md), and the authoritative WIT is [`crates/wasi-sql/wit/sql.wit`](../../crates/wasi-sql/wit/sql.wit).

- [Interfaces](#interfaces)
- [Statements and placeholders](#statements-and-placeholders)
- [Transactions](#transactions)
- [The portable dialect](#the-portable-dialect)
- [Portable limits](#portable-limits)
- [Values and column types](#values-and-column-types)
- [Errors](#errors)
- [The SDK's value mapping](#the-sdks-value-mapping)

## Interfaces

```wit
// store
open: async func(name: string) -> result<database, error>;

// database
resource database {
  query: async func(sql: string, params: list<option<value>>) -> result<result-set, error>;
  execute: async func(sql: string, params: list<option<value>>) -> result<u64, error>;
  batch: async func(statements: list<statement>) -> result<list<u64>, error>;
}

// transactions
begin: async func(database: borrow<database>) -> result<transaction, error>;

resource transaction {
  query: async func(sql: string, params: list<option<value>>) -> result<result-set, error>;
  execute: async func(sql: string, params: list<option<value>>) -> result<u64, error>;
  commit: async func() -> result<_, error>;
  rollback: async func() -> result<_, error>;
}
```

| Interface | Role |
| --------- | ---- |
| `types` | The `error`, `value`, `row`, and `result-set` types the others share. |
| `store` | `open` returns a handle on a database the deployment declares. An undeclared name fails with `not-found`. |
| `database` | Statements on one database. Each call commits before it returns. |
| `transactions` | Optional. Statements that commit or roll back together, over as many calls as it takes. |

The `core` world holds `types`, `store`, and `database`, which every backend serves. The `imports` world adds `transactions`, and a backend answers `unsupported` for what it does not serve.

- `query` returns a `result-set`: the selected `columns`, a `rows` stream, and a `finished` future the host always resolves. A query that selects no row returns its columns and no rows.
- A `row` holds one `option<value>` for each column, in select order, and `none` is `NULL`.
- `execute` returns how many rows a statement changed. A statement that changes no row, such as DDL, returns zero.
- `batch` runs its statements in order, in one transaction, and returns each one's changed-row count. A failing statement rolls back the whole batch, and the call fails with that statement's error. An empty batch returns an empty list.

A `database` handle holds the connection `open` returned until its first call. Every later call opens a connection for that call alone. Guests reach these interfaces through `omnia_sdk::sql`, which re-exports the bindings with helpers on the `database` and `transaction` handles, the `Executor` trait both implement, and `Database::transaction`, which commits a closure's work on `Ok` and rolls it back on `Err` (see the [guide](../guides/sql.md)).

## Statements and placeholders

A statement is one SQL statement, and text that holds more is `invalid`. It numbers its parameters `$1`, `$2`, and so on, skipping none, and `params` holds exactly one value for each. `$n` binds the `n`th value wherever it appears, in any order and as often as it appears. So `UPDATE items SET name = $2 WHERE id = $1` and `WHERE a = $1 OR b = $1` run alike on every backend. Other placeholder styles, such as `?`, are not portable.

`none` binds `NULL` to a parameter of any type. Whether session state, such as a temporary table, outlasts a call is the backend's to decide.

## Transactions

A transaction runs through these steps:

1. `begin` takes the `database` handle's connection, or opens one, and starts the transaction. The transaction stays open after the handle is dropped.
2. `query` and `execute` run on the transaction, one at a time, in the order made. Their statements follow the rules of `database`.
3. `commit` commits every change and ends the transaction. A commit that fails committed nothing, unless it failed with `unavailable`, when the outcome is unknown.
4. `rollback` discards every change and ends the transaction. Rolling back a transaction that already rolled back succeeds.

A failure on any call rolls the transaction back and ends it, and so does dropping it. A call on a transaction that has ended is `invalid`, and so is `rollback` after a commit.

A transaction holds one of the backend's connections until it ends. It never sees another transaction's uncommitted changes, and any stronger isolation is the backend's to give. A call outside the transaction may wait until it ends, so a guest that awaits one before ending the transaction may never finish. A backend may roll a transaction back early, when it is left idle or to let another proceed. The call this interrupts, or the next call, fails with `unavailable`.

The host owns transaction boundaries. It refuses, with `invalid`, a statement whose first keyword starts or ends a transaction or a savepoint, on `database` and on `transaction` alike:

- `BEGIN`, `START`, `COMMIT`, and `END`;
- `ROLLBACK` and `ABORT`;
- `SAVEPOINT` and `RELEASE`;
- `PREPARE` followed by `TRANSACTION`.

The check reads past leading whitespace, semicolons, and comments, and takes block comments as nested, the way Postgres does. Inside a transaction, a refused statement rolls the transaction back like any other failure.

`SET TRANSACTION` passes, so a guest sets an isolation level with a statement such as `SET TRANSACTION ISOLATION LEVEL SERIALIZABLE` as the transaction's first. Postgres serves it. SQLite has no `SET` statement, and its transactions are already serializable.

## The portable dialect

The host passes all other SQL through to the database. SQL outside the portable dialect still runs, but ties the guest to the database it was written for. The portable dialect is:

- The column types are Postgres names that SQLite accepts by affinity: `INTEGER`, `BIGINT`, `REAL`, `DOUBLE PRECISION`, `TEXT`, `BOOLEAN`, `BYTEA`, `DATE`, `TIME`, `TIMESTAMP`, and `TIMESTAMPTZ`.
- The caller supplies ids. A key the database generates, such as `SERIAL` or `AUTOINCREMENT`, is not portable.
- A statement that returns rows, such as one with a `RETURNING` clause, runs through `query`.
- `INSERT ... ON CONFLICT (...) DO NOTHING` and `DO UPDATE` run on both.

## Portable limits

Every backend accepts a portable name, statement, and row, and may reject any other as `invalid`:

- A database name is not empty, and holds only ASCII letters, digits, `_`, and `-`. Names that differ only in case may open the same database.
- A statement holds at most 64 KiB of SQL, with no NUL character, and binds at most 100 parameters.
- A row holds at most 1 MiB of values.
- A `text` value holds no NUL character, and a `float32` or `float64` is finite.

## Values and column types

Each `value` case binds to one column type. Bound to a parameter of another type, a value converts without loss, or the call fails with `invalid`. SQLite's parameters carry no type, so it stores a value as given. A temporal value must be in its case's format, or the call fails with `invalid`, naming the parameter:

- a `date` is `YYYY-MM-DD`;
- a `time` is `HH:MM:SS`, with optional fractional seconds;
- a `timestamp` is `YYYY-MM-DD HH:MM:SS`, with optional fractional seconds;
- a `timestamp-tz` is RFC 3339, and a portable one has a `+00:00` offset, such as `2024-01-02T03:04:05+00:00`.

A column reads back in a case that holds its value. Each backend returns these:

| Case | Column type | SQLite returns | Postgres returns |
| ---- | ----------- | -------------- | ---------------- |
| `boolean` | `BOOLEAN` | `int64` `0` or `1` | `boolean` |
| `int32` | `INTEGER` | `int64` | `int32` |
| `int64` | `BIGINT` | `int64` | `int64` |
| `float32` | `REAL` | `float64` | `float32` |
| `float64` | `DOUBLE PRECISION` | `float64` | `float64` |
| `text` | `TEXT` | `text` | `text` |
| `binary` | `BYTEA` | `binary` | `binary` |
| `date` | `DATE` | `text`, as bound | `date` |
| `time` | `TIME` | `text`, as bound | `time` |
| `timestamp` | `TIMESTAMP` | `text`, as bound | `timestamp` |
| `timestamp-tz` | `TIMESTAMPTZ` | `text`, as bound | `timestamp-tz`, with a `+00:00` offset |

So a portable reader takes:

- `int32` or `int64` from an integer column;
- `float32` or `float64` from a floating-point column;
- `boolean`, or `int64` `0` or `1`, from a `BOOLEAN` column;
- its own case, or `text` in that case's format, from a temporal column.

Precision finer than a microsecond is not portable, and fractional seconds may read back with trailing zeros added or dropped. Every backend reads a `NULL` back as `none`, whatever its column's type.

## Errors

| Case | Meaning |
| ---- | ------- |
| `not-found` | The deployment does not declare the database. A query that selects no row is not an error. |
| `denied` | The caller may not use the database, or lacks a privilege the statement needs. |
| `invalid` | The request was rejected, such as SQL that does not prepare, a parameter that does not bind, a statement the host refuses, or a call on an ended transaction. |
| `unsupported` | A valid request this backend cannot serve, such as a column type it cannot return, or `begin` on a backend that serves no transactions. |
| `unavailable` | The database did not answer, or the call exceeded its bound. Work the database rolls back to let another transaction proceed fails this way too. |
| `other` | Any other failure, such as a write a constraint refuses. |

The SDK reports each case as an `omnia_sdk::Error` coded with the case's name: a `BadGateway` for `unavailable`, and a `ServerError` for the rest.

## The SDK's value mapping

`omnia_sdk::sql::Value` is the WIT's cases under their own names, with `Null` for `none`. Each case binds the WIT case of its name, and `Null` binds `none`, which is `NULL`. A timestamp with a zone is a `TimestampTz`, and one without is a `Timestamp`, whatever its text holds.

`Value::from` builds the case a Rust value maps to, and builds `Null` from a `None`. `sql::params!` applies it to each of a statement's parameters. A `u32` builds an `Int64`, and a `u64` binds once the caller converts it to an `i64`.

It reads each case back into the case of its name, and `none` reads as `Null`, whatever its column's type. `Row::get` then decodes a column as a Rust type, whichever backend returned it:

- `i32` and `i64` from either integer case, in range;
- `f64` from `float32` or `float64`, and `f32` from `float32`;
- `bool` from `boolean`, or from an integer `0` or `1`;
- `String` from `text` or any temporal case;
- `Vec<u8>` or `Bytes` from `binary`;
- `Option<T>` reads `NULL` as `None`.
