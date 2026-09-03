//! The compiled guest components the e2e suites drive.
//!
//! The build script compiles every guest program in
//! `crates/test-programs/programs/<capability>/` to a `wasm32-wasip2`
//! component and generates one `pub const <NAME>: &str` path per program plus
//! a `foreach_<capability>!` macro; a suite invokes the macro to prove every
//! guest program has a matching test. The crate carries nothing else: a suite
//! runs an artifact through `omnia_test::host`.

include!(concat!(env!("OUT_DIR"), "/gen.rs"));
