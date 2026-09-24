//! Loading a name that is already active attests it: argv names a guest and
//! whether the deployment hashed its bytes. The handle carries the name and
//! either the recorded digest or none; nothing is acquired.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::plugins::{Plugins as _, WasiPlugins};

omnia_sdk::command!(scenario);

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let [_, name, hashed] = arguments.as_slice() else {
        panic!("expected `<name> <hashed|unhashed>`; got {arguments:?}");
    };

    let plugin = WasiPlugins.load(name).await.expect("an active guest attests");
    assert_eq!(plugin.id(), name);
    match hashed.as_str() {
        "hashed" => assert!(plugin.digest().is_some(), "the deployment hashed `{name}`"),
        "unhashed" => assert_eq!(plugin.digest(), None, "the deployment never hashed `{name}`"),
        other => panic!("expected `hashed` or `unhashed`, got `{other}`"),
    }
}
