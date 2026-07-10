//! Seam test for `wasi:messaging`: drive the `messaging` example guest's
//! publish path over the real `wasi:http` boundary and observe the message
//! arrive on a host-side subscription.
//!
//! `POST /pub-sub` makes the guest connect a client and `producer::send` the
//! body to topic `a`. A subscription taken on the shared backend *before* the
//! request then receives that message — proving the publish crossed the WIT
//! boundary into the host broker rather than merely returning `200`.
//!
//! The guest is built automatically on first [`find_guest`] call; the test skips locally when
//! it is absent and fails under CI so the pipeline never passes vacuously.

#![cfg(not(target_arch = "wasm32"))]

use std::time::Duration;

use anyhow::{Context as _, Result};
use futures::StreamExt as _;
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{Backend, HasHttp, Runtime};
use omnia_testkit::{http, single_guest};
use omnia_wasi_http::{HttpDefault, WasiHttp, WasiHttpCtxView};
use omnia_wasi_keyvalue::{HasKeyValue, KeyValueDefault, WasiKeyValue, WasiKeyValueCtx};
use omnia_wasi_messaging::{HasMessaging, MessagingDefault, WasiMessaging, WasiMessagingCtx};
use omnia_wasi_otel::{HasOtel, OtelDefault, WasiOtel, WasiOtelCtx};

/// The `examples/messaging` backend bundle: `wasi:http` + `wasi:otel` +
/// `wasi:messaging`, plus `wasi:keyvalue` — the guest's topic-`c` handler makes a
/// cached outbound call through `omnia_wasi_http::handle`, so the compiled guest
/// imports `wasi:keyvalue` regardless of which route the test drives.
#[derive(Clone)]
struct Bundle {
    http: HttpDefault,
    otel: OtelDefault,
    messaging: MessagingDefault,
    keyvalue: KeyValueDefault,
}

impl HasHttp for Bundle {
    fn http_view<'a>(&'a mut self, table: &'a mut ResourceTable) -> WasiHttpCtxView<'a> {
        self.http.as_view(table)
    }
}

impl HasOtel for Bundle {
    fn otel_ctx(&mut self) -> &mut dyn WasiOtelCtx {
        &mut self.otel
    }
}

impl HasMessaging for Bundle {
    fn messaging_ctx(&mut self) -> &mut dyn WasiMessagingCtx {
        &mut self.messaging
    }
}

impl HasKeyValue for Bundle {
    fn keyvalue_ctx(&mut self) -> &mut dyn WasiKeyValueCtx {
        &mut self.keyvalue
    }
}

/// Build the runtime, returning it plus a handle to the shared messaging backend
/// (its broadcast `sender` is shared across clones, so a subscription on this
/// handle observes the guest's publishes).
async fn runtime() -> Result<Option<(Runtime<Bundle>, MessagingDefault)>> {
    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
        otel: OtelDefault::connect().await.context("connecting otel")?,
        messaging: <MessagingDefault as Backend>::connect()
            .await
            .context("connecting messaging")?,
        keyvalue: KeyValueDefault::connect().await.context("connecting keyvalue")?,
    };
    let broker = bundle.messaging.clone();

    let Some(guest) = single_guest("messaging_wasm.wasm", bundle).await? else {
        return Ok(None);
    };
    let runtime = guest
        .host::<WasiHttp>()?
        .host::<WasiOtel>()?
        .host::<WasiMessaging>()?
        .host::<WasiKeyValue>()?
        .into_runtime()?;
    Ok(Some((runtime, broker)))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pub_sub() -> Result<()> {
    let Some((runtime, broker)) = runtime().await? else {
        return Ok(());
    };

    // Subscribe before publishing: the broadcast channel only delivers messages
    // sent after the receiver is taken.
    let client = broker.connect().await.context("connect broker client")?;
    let mut subscription = client.subscribe().await.context("subscribe")?;

    let response = http::post_json(&runtime, "/pub-sub", r#"{"hello":"world"}"#).await?;
    assert!(response.status().is_success(), "guest publishes across the messaging boundary");

    let message = tokio::time::timeout(Duration::from_secs(5), subscription.next())
        .await
        .context("timed out waiting for the published message")?
        .context("subscription closed without a message")?;

    assert_eq!(message.topic, "a", "guest published to topic `a`");
    assert_eq!(
        message.payload, br#"{"hello":"world"}"#,
        "the published payload reached the host broker intact"
    );

    Ok(())
}
