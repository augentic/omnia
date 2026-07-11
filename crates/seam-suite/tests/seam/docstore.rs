//! `wasi:docstore` seam: documents and the host-managed filter resource cross
//! the WIT boundary, and a probe on the shared backend proves mutations landed
//! host-side.

use anyhow::{Context as _, Result};
use omnia_testkit::http;
use omnia_wasi_docstore::WasiDocStoreCtx as _;

use crate::fixture::{self, unique};

#[test]
fn insert_then_get() -> Result<()> {
    fixture::RT.block_on(async {
        let fx = fixture::conformance().await?;
        let id = unique("stop");

        let create = http::post_json(
            &fx.runtime,
            "/docstore/stops",
            format!(r#"{{"id":"{id}","stop_name":"Central","zone_id":null}}"#),
        )
        .await?;
        assert!(create.status().is_success(), "guest inserts the document across the boundary");

        let fetched = http::get(&fx.runtime, &format!("/docstore/stops/{id}")).await?;
        assert!(fetched.status().is_success(), "guest reads the document back");
        let body: serde_json::Value = serde_json::from_slice(fetched.body())?;
        assert_eq!(body["id"], serde_json::json!(id), "the id round-trips");
        assert_eq!(
            body["stop"]["stop_name"],
            serde_json::json!("Central"),
            "the document round-trips"
        );

        // The insert must be visible on the shared backend.
        let stored = fx
            .docstore
            .get("stops".to_owned(), id.clone())
            .await
            .context("probe get")?
            .context("document missing from the host store")?;
        assert_eq!(stored.id, id, "the guest's insert reached the host store");

        Ok(())
    })
}

#[test]
fn filtered_query_then_delete() -> Result<()> {
    fixture::RT.block_on(async {
        let fx = fixture::conformance().await?;

        // Two stops in different zones: the filtered query must return only the
        // matching one, proving the filter resource crosses the WIT boundary
        // and is evaluated host-side.
        let zone = unique("zone");
        let in_zone = unique("stop-in");
        let out_of_zone = unique("stop-out");

        for (id, stop_zone) in [(&in_zone, zone.as_str()), (&out_of_zone, "other-zone")] {
            let create = http::post_json(
                &fx.runtime,
                "/docstore/stops",
                format!(r#"{{"id":"{id}","stop_name":"Stop {id}","zone_id":"{stop_zone}"}}"#),
            )
            .await?;
            assert!(create.status().is_success(), "guest inserts {id}");
        }

        let listed = http::get(&fx.runtime, &format!("/docstore/stops?zone={zone}")).await?;
        assert!(listed.status().is_success(), "guest queries with an eq filter");
        let body: serde_json::Value = serde_json::from_slice(listed.body())?;
        let stops = body["stops"].as_array().context("stops array")?;
        assert_eq!(stops.len(), 1, "only the matching zone is returned");
        assert_eq!(stops[0]["id"], serde_json::json!(in_zone));

        let deleted = http::delete(&fx.runtime, &format!("/docstore/stops/{in_zone}")).await?;
        assert!(deleted.status().is_success(), "guest deletes the document");

        // The delete must be visible on the shared backend.
        let stored = fx.docstore.get("stops".to_owned(), in_zone).await.context("probe get")?;
        assert!(stored.is_none(), "the delete reached the host store");

        Ok(())
    })
}
