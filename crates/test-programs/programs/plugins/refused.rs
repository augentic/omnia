//! A typed refusal at the wire: argv names a location, the `error` variant
//! its load must fail with, a needle the detail must carry, and optionally
//! the digest the load is pinned to — one program for the whole refusal
//! matrix. Bound directly to `omnia:plugins/loader`, so the variant is
//! checked as the host sent it and a malformed digest reaches the host as
//! written.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "requester",
    path: "wit",
    generate_all,
});

use omnia::plugins::loader::{self, Error, Location, RegistryRef};

omnia_sdk::command!(scenario);

// `declared`, `path`, `registry`, or `registry@<endpoint>` — the last naming
// the registry the load fetches from.
fn location(kind: &str, value: &str) -> Location {
    let value = value.to_owned();
    let (kind, endpoint) = kind
        .split_once('@')
        .map_or((kind, None), |(kind, endpoint)| (kind, Some(endpoint.to_owned())));
    match (kind, endpoint) {
        ("declared", None) => Location::Declared(value),
        ("path", None) => Location::Path(value),
        ("registry", endpoint) => Location::Registry(RegistryRef {
            package: value,
            endpoint,
        }),
        _ => panic!("expected `declared`, `path`, `registry`, or `registry@<endpoint>`"),
    }
}

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let (kind, value, variant, needle, digest) = match arguments.as_slice() {
        [_, kind, value, variant, needle] => (kind, value, variant, needle, None),
        [_, kind, value, variant, needle, digest] => {
            (kind, value, variant, needle, Some(digest.clone()))
        }
        _ => panic!(
            "expected `<declared|path|registry> <value> <refused|unavailable|internal> <needle> \
             [digest]`; got {arguments:?}"
        ),
    };

    let error = loader::load(location(kind, value), digest).await.expect_err("the load is refused");
    let (code, detail) = match &error {
        Error::Refused(detail) => ("refused", detail),
        Error::Unavailable(detail) => ("unavailable", detail),
        Error::Internal(detail) => ("internal", detail),
    };
    assert_eq!(code, variant, "{error:?}");
    assert!(detail.contains(needle.as_str()), "`{detail}` does not mention `{needle}`");
}
