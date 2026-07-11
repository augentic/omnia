//! `wasi:messaging` seam: the guest publishes to topic `a`, and a subscription
//! taken on the shared broker before the request receives that message.

use std::time::Duration;

use anyhow::{Context as _, Result};
use futures::StreamExt as _;
use omnia_testkit::http;
use omnia_wasi_messaging::WasiMessagingCtx as _;

use crate::fixture::{self, unique};

#[test]
fn pub_sub() -> Result<()> {
    fixture::RT.block_on(async {
        let fx = fixture::conformance().await?;
        let marker = unique("msg");
        let payload = format!(r#"{{"hello":"{marker}"}}"#);

        // Subscribe before publishing: the broadcast channel only delivers
        // messages sent after the receiver is taken.
        let client = fx.messaging.connect().await.context("connect broker client")?;
        let mut subscription = client.subscribe().await.context("subscribe")?;

        let response = http::post_json(&fx.runtime, "/messaging/pub-sub", payload.clone()).await?;
        assert!(response.status().is_success(), "guest publishes across the messaging boundary");

        // Other suite tests share the broker, so skip unrelated messages until
        // ours arrives.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let message = tokio::time::timeout_at(deadline, subscription.next())
                .await
                .context("timed out waiting for the published message")?
                .context("subscription closed without a message")?;
            if message.payload == payload.as_bytes() {
                assert_eq!(message.topic, "a", "guest published to topic `a`");
                return Ok(());
            }
        }
    })
}
