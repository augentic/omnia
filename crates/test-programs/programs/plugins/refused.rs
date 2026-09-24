//! A typed refusal at the wire: argv names a guest, the `error` variant its
//! load must fail with, and a needle the detail must carry — one program for
//! the whole refusal matrix. Bound directly to `omnia:plugins/loader`, so the
//! variant is checked as the host sent it.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "requester",
    path: "wit",
    generate_all,
});

use omnia::plugins::loader::{self, Error};

omnia_sdk::command!(scenario);

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let [_, name, variant, needle] = arguments.as_slice() else {
        panic!("expected `<name> <refused|unavailable|internal> <needle>`; got {arguments:?}");
    };

    let error = loader::load(name.clone()).await.expect_err("the load is refused");
    let (code, detail) = match &error {
        Error::Refused(detail) => ("refused", detail),
        Error::Unavailable(detail) => ("unavailable", detail),
        Error::Internal(detail) => ("internal", detail),
    };
    assert_eq!(code, variant, "{error:?}");
    assert!(detail.contains(needle.as_str()), "`{detail}` does not mention `{needle}`");
}
