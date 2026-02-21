# WASI SQL Implementation

This crate provides both guest and host implementations for the `wasi::sql` specification:

- **Guest**: WIT bindings and a type-safe ORM layer for WASM components
- **Host**: Runtime service with `SQLite` backend for wasmtime-based hosts

## Features

### Guest ORM Layer

The guest module provides query builders for type-safe database operations:

- **Entity macro**: Declare database models with automatic trait implementations
- **Query builders**: SELECT, INSERT, UPDATE, DELETE with fluent APIs
- **Joins**: Support for INNER, LEFT, RIGHT, and FULL joins
- **Filters**: Type-safe WHERE clauses with comparisons and logical operators
- **Column aliasing**: Manual column specifications for joined tables
- **Type conversion**: Automatic conversion between Rust types and SQL data types
- **Upserts**: Native INSERT ... ON CONFLICT support

### Host Implementation

The host provides a wasmtime component implementation using `SQLite`:

- **Multiple connections**: Named connection pools
- **Transactions**: Read-write transaction support
- **Prepared statements**: Efficient query execution
- **In-memory & file-based**: Supports both `:memory:` and file-backed databases

## Testing

### All together

```bash
# Run unit tests (entity module) and integration tests (orm, filter, entity) all together
cargo test -p omnia-wasi-sql --lib --target wasm32-wasip2 --no-run && \
  cargo test -p omnia-wasi-sql --test orm --target wasm32-wasip2 --no-run && \
  cargo test -p omnia-wasi-sql --test entity --target wasm32-wasip2 --no-run && \
  cargo test -p omnia-wasi-sql --test filter --target wasm32-wasip2 --no-run && \
  WASMTIME_BACKTRACE_DETAILS=1 wasmtime target/wasm32-wasip2/debug/deps/omnia_wasi_sql-*.wasm && \
  WASMTIME_BACKTRACE_DETAILS=1 wasmtime target/wasm32-wasip2/debug/deps/orm-*.wasm && \
  WASMTIME_BACKTRACE_DETAILS=1 wasmtime target/wasm32-wasip2/debug/deps/entity-*.wasm && \
  WASMTIME_BACKTRACE_DETAILS=1 wasmtime target/wasm32-wasip2/debug/deps/filter-*.wasm

```

### Individually

```bash
# Run unit tests (entity module) and integration tests (orm, filter, entity) individually
cargo test -p omnia-wasi-sql --lib --target wasm32-wasip2 --no-run &&
  WASMTIME_BACKTRACE_DETAILS=1 wasmtime target/wasm32-wasip2/debug/deps/omnia_wasi_sql-*.wasm
```

```bash
cargo test -p omnia-wasi-sql --test orm --target wasm32-wasip2 --no-run &&
  WASMTIME_BACKTRACE_DETAILS=1 wasmtime target/wasm32-wasip2/debug/deps/orm-*.wasm
```

```bash
cargo test -p omnia-wasi-sql --test entity --target wasm32-wasip2 --no-run &&
  WASMTIME_BACKTRACE_DETAILS=1 wasmtime target/wasm32-wasip2/debug/deps/entity-*.wasm
```

```bash
cargo test -p omnia-wasi-sql --test filter --target wasm32-wasip2 --no-run &&
  WASMTIME_BACKTRACE_DETAILS=1 wasmtime target/wasm32-wasip2/debug/deps/filter-*.wasm
```
