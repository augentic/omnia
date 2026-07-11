//! `wasi:vault` seam: the guest sets and reads back a secret, and a probe on
//! the shared backend proves the write crossed into the host vault.

use anyhow::{Context as _, Result};
use omnia_testkit::http;
use omnia_wasi_vault::WasiVaultCtx as _;

use crate::fixture::{self, unique};

#[test]
fn set_then_get() -> Result<()> {
    fixture::RT.block_on(async {
        let fx = fixture::conformance().await?;
        let secret_id = unique("secret");

        let response =
            http::post(&fx.runtime, &format!("/vault?secret={secret_id}"), r#"{"token":"s3cret"}"#)
                .await?;
        assert!(response.status().is_success(), "guest completes the vault round-trip");

        // The guest stored the body in `omnia-locker`; the shared backend must
        // now hold that write.
        let locker =
            fx.vault.open_locker("omnia-locker".to_owned()).await.context("open locker")?;
        let secret = locker.get(secret_id.clone()).await.context("read secret")?;
        assert_eq!(
            secret.as_deref(),
            Some(br#"{"token":"s3cret"}"#.as_slice()),
            "the secret reached the host vault"
        );

        // Lockers are isolated: the same id in another locker is absent.
        let other = fx.vault.open_locker(unique("locker")).await.context("open other locker")?;
        assert_eq!(
            other.get(secret_id).await.context("read other locker")?,
            None,
            "the secret is scoped to its locker"
        );

        Ok(())
    })
}
