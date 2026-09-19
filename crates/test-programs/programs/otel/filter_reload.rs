//! A filter reload reaches the live subscriber: `traced` is INFO-level, so
//! the guest's default `error` filter never creates its span, and only the
//! reload lets it through to the host.

#![cfg(target_arch = "wasm32")]

omnia_sdk::command!(scenario);

async fn scenario() {
    // Targeted rather than bare so an inherited `RUST_LOG` level (which wins
    // over the guest's own bare level) cannot disturb the outcome.
    omnia_wasi_otel::set_filter("otel_filter_reload=info").expect("filter reloads");
    traced().await;
}

#[omnia_wasi_otel::instrument]
async fn traced() {}
