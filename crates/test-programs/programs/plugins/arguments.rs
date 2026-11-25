//! Arguments: load the path the command's argv names, then ping the loaded
//! guest over the link. It answers with the argv it sees, which is empty:
//! only the command run receives the command's arguments.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "caller",
    path: "wit",
    generate_all,
});

use omnia_sdk::plugins::{self, File, Source};

use crate::omnia_test::link::ops;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let [_, path] = arguments.as_slice() else {
        panic!("expected `<path>`; got {arguments:?}");
    };
    let path = path.clone();
    let plugin = plugins::load(Source::Path(File { path, pin: None }))
        .await
        .expect("the load runs")
        .expect("the path loads");

    let answer = ops::ping(&plugin.name, "hi");
    assert_eq!(answer, format!("{} argv: []", plugin.name), "the loaded guest sees no arguments");
}
