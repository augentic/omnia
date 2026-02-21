# Examples

This directory contains examples demonstrating WASI capabilities with Omnia.

## Structure

Each example is comprised of a guest and a runtime. The guest is a WASI component (compiled to a `.wasm` file), while the runtime is a native binary that loads and executes the guest.

The runtime provides concrete implementations of WASI interfaces to connect the guest to backend services such as a key-value store, messaging, and a SQL database.

## Quick Start

Each example has a quick start guide in its README.


### Running Backend Services

TODO
