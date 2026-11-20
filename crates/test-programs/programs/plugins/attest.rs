//! Loading a name that is already active attests it: argv names a guest and
//! the digest the deployment recorded for its bytes at boot; the plugin
//! carries both back, and nothing is acquired.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::plugins::{self, Source};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let [_, name, recorded] = arguments.as_slice() else {
        panic!("expected `<name> <digest>`; got {arguments:?}");
    };

    let plugin = plugins::load(Source::Declared(name.clone()))
        .await
        .expect("the load runs")
        .expect("an active guest attests");
    assert_eq!(&plugin.name, name);
    assert_eq!(&plugin.digest, recorded, "the plugin attests the digest boot recorded");
}
