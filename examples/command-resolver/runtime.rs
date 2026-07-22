//! Resolver-backed command deployment, entirely macro-expressed.
//!
//! Every deployment key lands in one [`omnia::runtime!`] invocation: a static
//! guest from the inline manifest, a [`omnia::GuestResolver`] faulting further
//! guests in on registry misses, explicit command routing (`command_guest:`),
//! and raw argv passthrough (`program:`). Under `program:` there is no host
//! `run` grammar — the binary's argv belongs to the guest, so it runs as
//! `command-resolver greet Ada`, not `command-resolver run -- greet Ada`; see
//! `README.md`.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use omnia::futures::FutureExt as _;
        use omnia::{FutureResult, GuestArtifact, GuestId, GuestResolver};

        const GUEST_DIR: &str =
            concat!(env!("CARGO_MANIFEST_DIR"), "/../target/wasm32-wasip2/debug/examples");

        // Resolution is deployment policy: this deployment maps an identity to
        // a workspace-built component file and declines anything else. A real
        // embedder verifies (digest, signature, provenance) before returning
        // the bytes.
        struct DirResolver;

        impl GuestResolver for DirResolver {
            fn resolve(
                &self,
                guest: GuestId,
                _expected_export: String,
            ) -> FutureResult<Option<GuestArtifact>> {
                async move {
                    let path = std::path::Path::new(GUEST_DIR)
                        .join(format!("{}_wasm.wasm", guest.as_str().replace('-', "_")));
                    match tokio::fs::read(&path).await {
                        Ok(bytes) => Ok(Some(GuestArtifact::wasm(bytes))),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                        Err(error) => Err(error.into()),
                    }
                }
                .boxed()
            }
        }

        omnia::runtime!({
            mode: command,
            program: "command-resolver",
            guests: [
                { id: "cli", source: concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../target/wasm32-wasip2/debug/examples/cli_wasm.wasm",
                ) },
            ],
            resolver: DirResolver,
            command_guest: "cli",
        });
    } else {
        fn main() {}
    }
}
