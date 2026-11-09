//! `#[derive(HostCtx)]` compile-fail diagnostics.

#[test]
fn compile_fail_diagnostics() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/not_a_wasi_host.rs");
    cases.compile_fail("tests/ui/generic_host.rs");
}
