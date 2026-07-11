//! `wasi:blobstore` seam: the guest streams a blob out and back through a
//! container, and a probe on the shared backend proves the object landed in
//! the host store.

use anyhow::{Context as _, Result};
use omnia_testkit::http;
use omnia_wasi_blobstore::WasiBlobstoreCtx as _;

use crate::fixture::{self, unique};

#[test]
fn write_then_read() -> Result<()> {
    fixture::RT.block_on(async {
        let fx = fixture::conformance().await?;
        let object = unique("blob");

        let response = http::post(
            &fx.runtime,
            &format!("/blobstore?object={object}"),
            r#"{"blob":"payload"}"#,
        )
        .await?;
        assert!(response.status().is_success(), "guest completes the blob write/read round-trip");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(response.body())?,
            serde_json::json!({ "blob": "payload" }),
            "the guest echoes the blob it stored and read back"
        );

        // The blob written by the guest must be visible on the shared backend.
        let container =
            fx.blobstore.get_container("container".to_string()).await.context("probe container")?;
        let data = container
            .get_data(object.clone(), 0, 0)
            .await
            .context("probe object")?
            .with_context(|| format!("object `{object}` missing from the host store"))?;
        assert_eq!(
            data,
            br#"{"blob":"payload"}"#.as_slice(),
            "the guest's blob reached the host store intact"
        );

        Ok(())
    })
}
