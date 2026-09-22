//! The deployment's tracing level reaches the guest's WASI environment:
//! `RUST_LOG` arrives with the value the first argument names.

#![cfg(target_arch = "wasm32")]

omnia_sdk::command!(scenario);

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let [_, expected] = arguments.as_slice() else {
        panic!("expected the `RUST_LOG` value; got {arguments:?}");
    };

    let environment = wasip3::cli::environment::get_environment();
    let rust_log = environment.iter().find(|(name, _)| name == "RUST_LOG").map(|(_, value)| value);

    assert_eq!(rust_log, Some(expected), "the guest's `RUST_LOG` is the deployment's level");
}
