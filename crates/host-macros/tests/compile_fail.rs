//! Compile-fail suite: each case under `tests/compile_fail/` is an invalid
//! `runtime!` invocation whose spanned diagnostic is pinned by a `.stderr` file.

#[test]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}
