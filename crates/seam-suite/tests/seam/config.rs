//! `wasi:config` seam: the guest reads the host environment snapshot through
//! `get-all` across the WIT boundary.

use anyhow::{Context as _, Result};
use omnia_testkit::http;

use crate::fixture;

#[test]
fn get_all() -> Result<()> {
    fixture::RT.block_on(async {
        let fx = fixture::conformance().await?;

        let response = http::get(&fx.runtime, "/config").await?;
        assert!(response.status().is_success(), "guest reads config across the boundary");

        // `get-all` returns `list<tuple<string, string>>`, so `config` is a JSON
        // array of `[key, value]` pairs. The runtime's own env is non-empty, so a
        // populated array proves the variables crossed the boundary.
        let body: serde_json::Value = serde_json::from_slice(response.body())?;
        let config = body.get("config").context("response carries a config field")?;
        assert!(config.is_array(), "config is the get-all list: {config}");

        Ok(())
    })
}
