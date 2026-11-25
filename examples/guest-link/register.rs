//! Dynamic guest registration: grow the deployment after startup.
//!
//! Boots the two-guest `guest-link` deployment, then registers a third guest
//! (`extra`, absent from the manifest) at run time via `Runtime::register`.
//! The `router` reaches it through the same host-mediated link dispatch as
//! any manifest target — `run-to("extra", ...)` — so a guest registered at
//! run time is reachable end to end. The manifest names its guests by path,
//! so each loads at its first use: the router when `call_router` first
//! resolves it, and `responder` inside the relay when the router first calls
//! it. Build the guests first (see README.md), then:
//!
//!   cargo run --example guest-link-register

cfg_select! {
    not(target_arch = "wasm32") => {
        use std::path::Path;

        use anyhow::{Context as _, Result, bail};
        use omnia::wasmtime::component::Val;
        use omnia::{DeploymentBuilder, Grants, GuestEntry, GuestId, Manifest, Runtime};

        #[tokio::main]
        async fn main() -> Result<()> {
            let artifacts = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../target/wasm32-wasip3/debug/examples");

            let manifest = Manifest::new()
                .guest(GuestEntry::new(
                    "responder",
                    artifacts.join("guest_link_responder_wasm.wasm"),
                ))
                .guest(GuestEntry::new("router", artifacts.join("guest_link_router_wasm.wasm")));

            let deployment = DeploymentBuilder::new()
                .manifest(manifest)
                .build::<()>()
                .await
                .context("building deployment")?;
            let runtime = deployment.assemble(()).await?;

            // absent from the manifest; an install pipeline would verify the
            // bytes first, here the "install" is a file read
            let wasm = std::fs::read(artifacts.join("guest_link_extra_wasm.wasm")).context(
                "extra guest not built: cargo build -p examples --example \
                 guest-link-extra-wasm --target wasm32-wasip3",
            )?;
            runtime.register("extra", wasm, Grants::all()).await?;

            // dispatched exactly as a manifest guest; the second call loads `responder` on the way
            println!("{}", call_router(&runtime, "extra", "hello").await?);
            println!("{}", call_router(&runtime, "responder", "hello").await?);

            runtime.deregister(&GuestId::from("extra"))?;
            Ok(())
        }

        // loads the router from the manifest's path on the first call
        async fn call_router(runtime: &Runtime<()>, target: &str, message: &str) -> Result<String> {
            let guest = runtime
                .guest(&GuestId::from("router"))
                .await
                .context("resolving the router guest")?;
            let results = runtime
                .lease(guest)
                .await
                .invoke(async |mut entered, instance, _bound| {
                    let run_to = instance
                        .get_func(&mut entered, "run-to")
                        .context("router exports `run-to`")?;

                    let mut results = vec![Val::Bool(false)];
                    run_to
                        .call_async(
                            &mut entered,
                            &[Val::String(target.to_owned()), Val::String(message.to_owned())],
                            &mut results,
                        )
                        .await
                        .map_err(anyhow::Error::from)
                        .context("calling router.run-to")?;
                    anyhow::Ok(results)
                })
                .await
                .context("instantiating router")??;

            match results.into_iter().next() {
                Some(Val::String(echoed)) => Ok(echoed),
                other => bail!("router.run-to returned a non-string result: {other:?}"),
            }
        }
    }
    _ => {
        fn main() {}
    }
}
