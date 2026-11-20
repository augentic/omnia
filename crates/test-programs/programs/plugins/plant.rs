//! A guest plants the file a declared path entry reads at its first use:
//! argv names the payload to copy, the path to plant it at beneath the
//! writable `.` mount, and the outcome — a code with a needle the detail
//! must carry, or `loaded`. A pre-compiled payload is `invalid` because the
//! entry is unpinned and read while guests run; raw wasm loads, since the
//! host compiles it itself.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::plugins::{self, Source};
use test_programs::fails_with;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let (payload, target, refusal) = match arguments.as_slice() {
        [_, payload, target, outcome] if outcome == "loaded" => (payload, target, None),
        [_, payload, target, code, needle] => (payload, target, Some((code, needle))),
        _ => panic!(
            "expected `<payload> <target> <code> <needle>` or `<payload> <target> loaded`; got \
             {arguments:?}"
        ),
    };

    // The entry's `source.path` is read only when the guest asks for it, so
    // whatever sits there by then is what the host is handed.
    std::fs::copy(payload, target).expect("the payload is planted beneath the `.` mount");

    let loaded = plugins::load(Source::Declared("plugin".to_owned())).await;
    match (loaded, refusal) {
        (Err(error), Some((code, needle))) => fails_with(&error, code, needle),
        (Ok(Ok(plugin)), None) => assert_eq!(plugin.name, "plugin"),
        (loaded, Some((code, needle))) => {
            panic!("expected `{code}` mentioning `{needle}`; got {loaded:?}")
        }
        (loaded, None) => panic!("expected the planted wasm to load; got {loaded:?}"),
    }
}
