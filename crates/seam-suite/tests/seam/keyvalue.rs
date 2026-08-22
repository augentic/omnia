//! `wasi:keyvalue` seam: the guest's open/set/get and CAS legs cross the WIT
//! boundary, and a probe on the shared backend proves the writes landed
//! host-side.

use anyhow::{Context as _, Result};
use omnia_testkit::http;
use omnia_wasi_keyvalue::WasiKeyValueCtx as _;

use crate::fixture::{self, unique};

#[test]
fn set_then_get() -> Result<()> {
    fixture::RT.block_on(async {
        let fx = fixture::conformance().await?;
        let key = unique("kv-set");
        let cas = unique("kv-set-cas");

        let response =
            http::post(&fx.runtime, &format!("/keyvalue?key={key}&cas={cas}"), "payload-value")
                .await?;
        assert!(response.status().is_success(), "guest completes the keyvalue round-trip");

        // The guest stored the request body under `key` in `omnia_bucket`; the
        // shared backend must now hold that write.
        let bucket =
            fx.keyvalue.open_bucket("omnia_bucket".to_owned()).await.context("open bucket")?;
        let stored = bucket.get(key).await.context("read key")?;
        assert_eq!(
            stored.as_deref(),
            Some(b"payload-value".as_slice()),
            "the write reached the host"
        );

        Ok(())
    })
}

#[test]
fn cas_then_increment() -> Result<()> {
    fixture::RT.block_on(async {
        let fx = fixture::conformance().await?;
        let key = unique("state-cas");
        let counter = unique("state-counter");

        // The guest drives `StateStore` through absent-expected cas (create),
        // matching cas (swap), stale cas (typed `CasError::Conflict` asserted
        // guest-side and reported here), then increments by 5 and -2.
        let response =
            http::post(&fx.runtime, &format!("/state?key={key}&counter={counter}"), "state-seed")
                .await?;
        assert!(response.status().is_success(), "guest completes the state legs");
        let report: serde_json::Value =
            serde_json::from_slice(response.body()).context("parsing the state report")?;
        assert_eq!(report["conflict"], "swapped", "the conflict carried the observed value");
        assert_eq!(report["counter"], 3, "increments sum guest-side");

        // The StateStore defaults act on the `cache` bucket; the stale cas
        // must have left the matching swap's value in place.
        let bucket = fx.keyvalue.open_bucket("cache".to_owned()).await.context("open bucket")?;
        let stored = bucket.get(key).await.context("read state key")?;
        assert_eq!(
            stored.as_deref(),
            Some(b"swapped".as_slice()),
            "the swap landed and the stale cas changed nothing"
        );
        let count = bucket.get(counter).await.context("read counter")?;
        assert_eq!(
            count.as_deref(),
            Some(3_i64.to_be_bytes().as_slice()),
            "the increments landed host-side"
        );

        Ok(())
    })
}

#[test]
fn cas_swap() -> Result<()> {
    fixture::RT.block_on(async {
        let fx = fixture::conformance().await?;
        let key = unique("kv-cas");
        let cas = unique("kv-cas-key");

        // The guest exercises both CAS legs: a clean swap, then a stale swap
        // whose `cas-failed` handle is retried. A success response means every
        // leg behaved per the WIT contract.
        let response =
            http::post(&fx.runtime, &format!("/keyvalue?key={key}&cas={cas}"), "cas-seed").await?;
        assert!(response.status().is_success(), "guest completes the CAS round-trip");

        // The retry with the refreshed handle is the last write to land host-side.
        let bucket =
            fx.keyvalue.open_bucket("omnia_bucket".to_owned()).await.context("open bucket")?;
        let stored = bucket.get(cas).await.context("read cas key")?;
        assert_eq!(
            stored.as_deref(),
            Some(b"retried".as_slice()),
            "the retried swap reached the host"
        );

        Ok(())
    })
}
