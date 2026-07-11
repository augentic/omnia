//! `wasi:websocket` seam, both directions: the guest's `connect` + `send`
//! crosses into the host and reaches a connected external peer, and a peer
//! message travels back through the host into the guest's event handler.

use std::time::Duration;

use anyhow::{Context as _, Result};
use futures::{SinkExt as _, StreamExt as _};
use omnia_testkit::http;
use omnia_wasi_keyvalue::WasiKeyValueCtx as _;
use tokio_tungstenite::tungstenite::Message;

use crate::fixture;

#[test]
fn send_reaches_connected_peer() -> Result<()> {
    fixture::RT.block_on(async {
        let fx = fixture::conformance().await?;

        // The backend's server starts on a spawned task; retry until it accepts.
        let url = format!("ws://127.0.0.1:{}", fx.websocket_port);
        let mut peer = None;
        for _ in 0..50 {
            match tokio_tungstenite::connect_async(&url).await {
                Ok((stream, _)) => {
                    peer = Some(stream);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
        let mut peer = peer.context("websocket server did not accept a connection")?;

        // The guest's send only reaches peers registered before it fires; the
        // handshake and registration race, so retry the request until delivery.
        let mut delivered = None;
        for _ in 0..10 {
            let response = http::post(&fx.runtime, "/websocket", "hello sockets").await?;
            assert!(
                response.status().is_success(),
                "guest connects and sends across the ws boundary"
            );
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(response.body())?,
                serde_json::json!({ "message": "event sent" }),
                "guest acknowledges the send it drove through the host"
            );

            match tokio::time::timeout(Duration::from_secs(1), peer.next()).await {
                Ok(message) => {
                    delivered = Some(message.context("connection closed without a message")??);
                    break;
                }
                Err(_elapsed) => {}
            }
        }

        let message = delivered.context("guest event never reached the connected peer")?;
        assert_eq!(
            message,
            Message::Binary(b"hello sockets".as_slice().into()),
            "the guest's payload reached the external peer intact"
        );

        // Inbound leg: the peer's message must cross host -> guest handler,
        // which records it in the shared keyvalue bucket.
        peer.send(Message::Binary(b"ping from peer".as_slice().into()))
            .await
            .context("peer send")?;
        let bucket =
            fx.keyvalue.open_bucket("omnia_bucket".to_owned()).await.context("open bucket")?;
        let mut recorded = None;
        for _ in 0..50 {
            if let Some(value) = bucket.get("ws-inbound".to_owned()).await.context("probe")? {
                recorded = Some(value);
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(
            recorded.as_deref(),
            Some(b"ping from peer".as_slice()),
            "the peer's message reached the guest handler"
        );

        Ok(())
    })
}
