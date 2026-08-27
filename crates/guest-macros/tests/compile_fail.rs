//! Compile-fail suite: each case under `tests/compile_fail/` is an invalid
//! `#[handler]` function whose spanned diagnostic is pinned by a `.stderr`
//! file.

#[test]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}
