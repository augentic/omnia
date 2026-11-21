//! The offline WIT gate over the tree as committed.

use std::process::Command;

#[test]
fn tree() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["check", test_programs::CONFIG_SEEDED])
        .output()
        .expect("xtask should run");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}
