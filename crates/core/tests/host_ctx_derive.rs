//! `#[derive(HostCtx)]` compile-fail diagnostics.

#[test]
fn compile_fail_diagnostics() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/missing_client.rs");
    cases.compile_fail("tests/ui/generic_host.rs");
}
