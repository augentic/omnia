//! A typed failure at the wire: argv names a source, the case its load must
//! answer, a needle the detail must carry, and optionally the digest the
//! source is pinned to — one program for the whole failure matrix. Bound
//! directly to `omnia:plugins`, so the case is checked as the host sent it
//! and a malformed digest reaches the host as written. A conflict is the
//! case `conflict`, whose detail reads `found=<digest> current=<digest|none>`.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "requester",
    path: "wit",
    generate_all,
});

use omnia::plugins::loader;
use omnia::plugins::types::{Error, File, Release, Source};

omnia_sdk::api::command::export!(scenario);

// `declared`, `path`, `release`, or `release@<registry>` — the last naming
// the registry the load fetches from. A declared source carries no pin.
fn source(kind: &str, value: &str, pin: Option<String>) -> Source {
    let value = value.to_owned();
    let (kind, registry) = kind
        .split_once('@')
        .map_or((kind, None), |(kind, registry)| (kind, Some(registry.to_owned())));
    match (kind, registry) {
        ("declared", None) if pin.is_none() => Source::Declared(value),
        ("path", None) => Source::Path(File { path: value, pin }),
        ("release", registry) => Source::Release(Release {
            reference: value,
            registry,
            pin,
        }),
        _ => panic!("expected `declared`, `path`, `release`, or `release@<registry>`"),
    }
}

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let (kind, value, case, needle, pin) = match arguments.as_slice() {
        [_, kind, value, case, needle] => (kind, value, case, needle, None),
        [_, kind, value, case, needle, pin] => (kind, value, case, needle, Some(pin.clone())),
        _ => panic!(
            "expected `<declared|path|release> <value> <not-found|denied|invalid|unsupported|\
             unavailable|other|conflict> <needle> [pin]`; got {arguments:?}"
        ),
    };

    let (answered, detail) = match loader::load(source(kind, value, pin)).await {
        Ok(Ok(plugin)) => panic!("`{}` loaded, not a `{case}` failure", plugin.name),
        Ok(Err(conflict)) => {
            let current =
                conflict.current.map_or_else(|| "none".to_owned(), |plugin| plugin.digest);
            ("conflict", format!("found={} current={current}", conflict.found))
        }
        Err(Error::NotFound(detail)) => ("not-found", detail),
        Err(Error::Denied(detail)) => ("denied", detail),
        Err(Error::Invalid(detail)) => ("invalid", detail),
        Err(Error::Unsupported(detail)) => ("unsupported", detail),
        Err(Error::Unavailable(detail)) => ("unavailable", detail),
        Err(Error::Other(detail)) => ("other", detail),
    };
    assert_eq!(answered, case, "{detail}");
    assert!(detail.contains(needle.as_str()), "`{detail}` does not mention `{needle}`");
}
