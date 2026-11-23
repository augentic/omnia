//! The guest reaches what its grants list: each `NAME=value` argument is a
//! variable it sees, every other argument a mount it sees, and it sees no
//! other variable or mount.

#![cfg(target_arch = "wasm32")]

use std::collections::{BTreeMap, BTreeSet};

use wasip3::filesystem::preopens;

omnia_sdk::command::export!(scenario);

// `RUST_LOG` always, and `NO_COLOR` when the console is plain
const SET_BY_RUNTIME: [&str; 2] = ["RUST_LOG", "NO_COLOR"];

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let (pairs, mounts): (Vec<&String>, Vec<&String>) =
        arguments.iter().skip(1).partition(|argument| argument.contains('='));
    let expected_env: BTreeMap<&str, &str> =
        pairs.iter().filter_map(|pair| pair.split_once('=')).collect();
    let expected_mounts: BTreeSet<&str> = mounts.iter().map(|mount| mount.as_str()).collect();

    let environment = wasip3::cli::environment::get_environment();
    let env: BTreeMap<&str, &str> = environment
        .iter()
        .filter(|(name, _)| !SET_BY_RUNTIME.contains(&name.as_str()))
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(env, expected_env, "the guest sees the variables its grants list");

    let directories = preopens::get_directories();
    let preopened: BTreeSet<&str> = directories.iter().map(|(_, name)| name.as_str()).collect();
    assert_eq!(preopened, expected_mounts, "the guest sees the mounts its grants list");
}
