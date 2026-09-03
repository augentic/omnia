//! Compiles every guest program under `programs/<capability>/` to a
//! `wasm32-wasip2` component and generates `gen.rs`: one path constant per
//! program plus a `foreach_<capability>!` completeness macro per capability
//! directory, for the native side of this crate to `include!`.
//!
//! The nested build compiles this same package for `wasm32`, running this
//! script again; `Components` is a no-op under that target, so the recursion
//! stops there.

fn main() {
    omnia_test::build::Components::in_workspace("../..")
        .package("test-programs")
        .scan("crates/test-programs/programs")
        .sync_examples("crates/test-programs/Cargo.toml")
        // The guests' WIT lives outside the dep-info the nested build emits
        // (proc macros don't track file reads), so watch it explicitly.
        .track([
            "crates/test-programs/wit",
            "crates/test-programs/src",
            "crates/test-programs/Cargo.toml",
        ])
        .build()
        .write_gen("gen.rs");
}
