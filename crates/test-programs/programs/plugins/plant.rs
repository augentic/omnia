//! A guest plants the file a declared on-demand entry reads: argv names the
//! payload to copy, the path to plant it at beneath the writable `.` mount,
//! and the outcome — `refused` with a needle the detail must carry, or
//! `loaded`. A pre-compiled payload is refused because the entry is unpinned
//! and read on demand; raw wasm loads, since the host compiles it itself.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::plugins::{Error, Location, Plugins as _, WasiPlugins};

omnia_sdk::command!(scenario);

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let (payload, target, needle) = match arguments.as_slice() {
        [_, payload, target, outcome, needle] if outcome == "refused" => {
            (payload, target, Some(needle))
        }
        [_, payload, target, outcome] if outcome == "loaded" => (payload, target, None),
        _ => panic!(
            "expected `<payload> <target> refused <needle>` or `<payload> <target> loaded`; got \
             {arguments:?}"
        ),
    };

    // The entry's `source.path` is read only when the guest asks for it, so
    // whatever sits there by then is what the host is handed.
    std::fs::copy(payload, target).expect("the payload is planted beneath the `.` mount");

    let result = WasiPlugins.load(&Location::Declared("plugin".to_owned()), None).await;
    match (&result, needle) {
        (Err(Error::Refused(detail)), Some(needle)) => {
            assert!(detail.contains(needle.as_str()), "`{detail}` does not mention `{needle}`");
        }
        (Ok(plugin), None) => assert_eq!(plugin.id(), "plugin"),
        (_, Some(needle)) => panic!("expected a refusal mentioning `{needle}`; got {result:?}"),
        (_, None) => panic!("expected the planted wasm to load; got {result:?}"),
    }
}
