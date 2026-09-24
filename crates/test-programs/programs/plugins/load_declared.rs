//! A `declared` location through the omnia-sdk plugin requester: argv names a
//! guest and what to expect of loading it by name. `attests` — the handle
//! carries the name and, for a deployment guest, no digest; `undeclared` —
//! the load is refused, naming the guest; `pinned` — a declared name takes
//! no pin.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::plugins::{Error, Location, PluginRef, Plugins as _, WasiPlugins};

omnia_sdk::command!(scenario);

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let [_, name, expectation] = arguments.as_slice() else {
        panic!("expected `<name> <attests|undeclared|pinned>`; got {arguments:?}");
    };

    let declared = |digest| PluginRef {
        location: Location::Declared(name.clone()),
        digest,
    };
    match expectation.as_str() {
        "attests" => {
            let plugin = WasiPlugins.load(&declared(None)).await.expect("a declared guest attests");
            assert_eq!(plugin.id(), name);
            assert_eq!(plugin.digest(), None, "a deployment guest recorded no digest");
        }
        "undeclared" => {
            let err = WasiPlugins.load(&declared(None)).await.expect_err("an undeclared name");
            match err {
                Error::Refused(detail) => {
                    assert_eq!(detail, format!("no guest `{name}` is declared by this deployment"));
                }
                other => panic!("expected refused: {other:?}"),
            }
        }
        "pinned" => {
            let pin = format!("sha256:{}", "ab".repeat(32)).parse().expect("a well-formed pin");
            let err = WasiPlugins.load(&declared(Some(pin))).await.expect_err("a pinned name");
            match err {
                Error::Refused(detail) => {
                    assert_eq!(
                        detail,
                        format!("`{name}` is declared by the deployment; it takes no pin")
                    );
                }
                other => panic!("expected refused: {other:?}"),
            }
        }
        other => panic!("unknown expectation `{other}`"),
    }
}
