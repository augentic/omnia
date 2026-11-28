//! The guest reads `marker` through its `data` mount: argv names the
//! contents it must find there.

#![cfg(target_arch = "wasm32")]

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let [_, expected] = arguments.as_slice() else {
        panic!("expected `<contents>`; got {arguments:?}");
    };

    let marker = std::fs::read_to_string("data/marker").expect("reading `marker` through `data`");
    assert_eq!(&marker, expected, "the `data` mount is the directory it opened at startup");
}
