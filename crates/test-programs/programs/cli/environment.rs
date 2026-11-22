//! The guest's environment is the host variables the deployment names, beside
//! the ones the runtime sets: each argument is one `NAME=value` pair the
//! guest sees, and it sees no other.

#![cfg(target_arch = "wasm32")]

use std::collections::BTreeMap;

omnia_sdk::command::export!(scenario);

// `RUST_LOG` always, and `NO_COLOR` when the console is plain
const SET_BY_RUNTIME: [&str; 2] = ["RUST_LOG", "NO_COLOR"];

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let expected: BTreeMap<&str, &str> = arguments
        .iter()
        .skip(1)
        .map(|pair| pair.split_once('=').unwrap_or_else(|| panic!("`{pair}` is not `NAME=value`")))
        .collect();

    let environment = wasip3::cli::environment::get_environment();
    let inherited: BTreeMap<&str, &str> = environment
        .iter()
        .filter(|(name, _)| !SET_BY_RUNTIME.contains(&name.as_str()))
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();

    assert_eq!(inherited, expected, "the guest sees the host variables the deployment names");
    assert!(environment.iter().any(|(name, _)| name == "RUST_LOG"), "the runtime sets `RUST_LOG`");
}
