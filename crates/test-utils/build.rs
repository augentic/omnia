//! Compiles every `crates/test-programs` guest program to a `wasm32-wasip2`
//! component and generates `gen.rs`: one path constant per program plus a
//! `foreach_<capability>!` completeness macro per capability directory.

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
