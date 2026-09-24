//! Compiles the guests the macro-expressed hosts embed — `cli-static` and
//! `guest-link` — to `wasm32-wasip2` components, and names each artifact in
//! an environment variable (`CLI_WASM`, `GUEST_LINK_RESPONDER_WASM`,
//! `GUEST_LINK_ROUTER_WASM`) the host's `guests:` entry reads with `env!`,
//! so those examples build and run with no guest build step first.
//!
//! The nested build compiles this same package for `wasm32`, running this
//! script again; `Components` is a no-op under that target, so the recursion
//! stops there. The other examples read their guests at start from
//! `target/wasm32-wasip2` and are built as their READMEs say.

fn main() {
    let built = omnia_test::build::Components::in_workspace("..")
        .package("examples")
        .examples(["cli-wasm", "guest-link-responder-wasm", "guest-link-router-wasm"])
        .build();
    for program in built.programs() {
        println!("cargo:rustc-env={}={}", program.constant, built.artifact(program).display());
    }
}
