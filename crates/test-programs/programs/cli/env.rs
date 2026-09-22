//! The deployment's `[env]` defaults reach the guest's WASI environment:
//! `OMNIA_TEST_DEFAULT` arrives with the value the manifest declared, while
//! the variable named by the first argument — one the host process sets, and
//! the manifest also defaults — arrives with the host's value, given as the
//! second argument.

#![cfg(target_arch = "wasm32")]

omnia_sdk::command!(scenario);

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let [_, shadowed, host_value] = arguments.as_slice() else {
        panic!("expected the shadowed variable's name and host value; got {arguments:?}");
    };

    let environment = wasip3::cli::environment::get_environment();
    let lookup = |name: &str| {
        environment.iter().find(|(set, _)| set == name).map(|(_, value)| value.as_str())
    };

    assert_eq!(lookup("OMNIA_TEST_DEFAULT"), Some("from-manifest"));
    assert_eq!(lookup(shadowed), Some(host_value.as_str()), "the host's value wins");
}
