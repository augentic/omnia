# Examples

This directory contains examples demonstrating WASI capabilities with Omnia.

## Structure

Each example is comprised of a **Guest** and a **Runtime**:

- **Guest**: A WASI component (compiled to a `.wasm` file) that contains the business logic.
- **Runtime**: A native Rust binary that loads the guest and provides the necessary host capabilities (like database access or network I/O).

## Quick Start

Navigate to any example directory and follow the instructions in its `README.md`.

Common examples include:
- **`http-hello`**: Basic HTTP server.
- **`keyvalue`**: Storing and retrieving state.
- **`messaging`**: Pub/Sub messaging.

## Running Backend Services

By default, the examples in this repository use **in-memory** implementations for services like Key-Value and Messaging. This means you can run them immediately without setting up external infrastructure (like Redis or NATS).

- **Key-Value**: Uses an in-memory cache. Data is lost when the runtime stops.
- **Messaging**: Uses in-memory broadcast channels. Messages are only delivered to subscribers within the same process.
- **SQL**: Uses SQLite (often in-memory or a local file).

### Production Backends

In a production environment, you would swap these default implementations for robust backends. The Omnia architecture allows you to change the host implementation without recompiling the guest.

For example, to use Redis for Key-Value, you would update the runtime configuration to use the Redis provider instead of the default in-memory one.
