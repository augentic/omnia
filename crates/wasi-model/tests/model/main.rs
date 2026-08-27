//! Public-API tests for `omnia-wasi-model`.

#[cfg(not(target_arch = "wasm32"))]
mod host;

mod prompt;

#[cfg(target_arch = "wasm32")]
mod guest;
