//! Loading a name that is already active attests it: argv names a guest and
//! the digest the deployment recorded for its bytes at boot; the handle
//! carries both back, and nothing is acquired.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::plugins::{Digest, Location, Plugins as _, WasiPlugins};

omnia_sdk::command!(scenario);

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let [_, name, recorded] = arguments.as_slice() else {
        panic!("expected `<name> <digest>`; got {arguments:?}");
    };
    let recorded = recorded.parse::<Digest>().expect("the recorded digest");

    let from = Location::Declared(name.clone());
    let plugin = WasiPlugins.load(&from, None).await.expect("an active guest attests");
    assert_eq!(plugin.id(), name);
    assert_eq!(plugin.digest(), &recorded, "the handle attests the digest boot recorded");
}
