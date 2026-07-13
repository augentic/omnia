//! The consolidated seam suite: every WASI interface exercised end-to-end
//! through a pre-built guest and the real WIT boundary, in one process.
//!
//! Most scenarios drive the shared [`fixture::conformance`] runtime — one
//! purpose-built guest (`examples/conformance`) with a route per HTTP-driven
//! interface, over one all-interface backend bundle, so the component, linker,
//! and `InstancePre` are created once per suite run. Scenarios that need a
//! different shape (CLI command mode, model replay, multi-guest routing,
//! guest-to-guest linking, MCP, the typed guest API) keep specialized fixtures
//! in their modules.
//!
//! Guests are never compiled from tests: build them first with
//! `cargo make test-guests` (a missing artifact fails fast with that
//! instruction).

#![cfg(not(target_arch = "wasm32"))]

mod fixture;

mod blobstore;
mod cli;
mod config;
mod docstore;
mod guest_api;
mod guest_link;
mod http;
mod identity;
mod keyvalue;
mod mcp;
mod messaging;
mod model;
mod otel;
mod routing;
mod sql;
mod vault;
mod websocket;
