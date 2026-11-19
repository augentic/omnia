//! Every package that uses `omnia:types` resolves it from a copy in its own
//! `wit/deps`, and this crate's `wit/types.wit` stays canonical.

#![cfg(not(target_arch = "wasm32"))]

// a drifted copy still resolves on its own, and fails only once a guest
// links two packages that disagree
#[test]
fn wit_copies() {
    let canonical = include_str!("../wit/types.wit");
    let copies = [
        ("plugin", include_str!("../../plugin/wit/deps/types.wit")),
        ("test-programs", include_str!("../../test-programs/wit/deps/types.wit")),
        ("wasi-blobstore", include_str!("../../wasi-blobstore/wit/deps/types.wit")),
        ("wasi-config", include_str!("../../wasi-config/wit/deps/types.wit")),
        ("wasi-docstore", include_str!("../../wasi-docstore/wit/deps/types.wit")),
        ("wasi-identity", include_str!("../../wasi-identity/wit/deps/types.wit")),
        ("wasi-keyvalue", include_str!("../../wasi-keyvalue/wit/deps/types.wit")),
        ("wasi-messaging", include_str!("../../wasi-messaging/wit/deps/types.wit")),
        ("wasi-model", include_str!("../../wasi-model/wit/deps/types.wit")),
        ("wasi-otel", include_str!("../../wasi-otel/wit/deps/types.wit")),
        ("wasi-sql", include_str!("../../wasi-sql/wit/deps/types.wit")),
        ("wasi-vault", include_str!("../../wasi-vault/wit/deps/types.wit")),
        ("wasi-vcs", include_str!("../../wasi-vcs/wit/deps/types.wit")),
        ("wasi-websocket", include_str!("../../wasi-websocket/wit/deps/types.wit")),
    ];
    for (krate, copy) in copies {
        assert_eq!(
            copy, canonical,
            "crates/{krate}/wit/deps/types.wit drifted from crates/wasi-types/wit/types.wit"
        );
    }
}
