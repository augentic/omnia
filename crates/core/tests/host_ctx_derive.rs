//! Compile-fail diagnostics: `#[derive(HostCtx)]`, a backend naming a host it
//! cannot serve, and a host linked without its host side.

#[test]
fn compile_fail_diagnostics() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/missing_client.rs");
    cases.compile_fail("tests/ui/generic_host.rs");
    cases.compile_fail("tests/ui/unserved_host.rs");
    cases.compile_fail("tests/ui/no_host_face.rs");
}
