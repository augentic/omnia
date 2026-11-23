//! Compile-fail diagnostics of `runtime!`, `#[derive(FromEnv)]`, and
//! `#[derive(HostCtx)]`.

#[test]
fn compile_fail_diagnostics() {
    trybuild::TestCases::new().compile_fail("tests/ui/*.rs");
}
