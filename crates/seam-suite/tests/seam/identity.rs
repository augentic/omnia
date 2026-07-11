//! `wasi:identity` seam: `get-identity` and `get-token` cross the boundary
//! against the credential-free `IdentityStub` and return its fixed token.

use anyhow::Result;
use omnia_testkit::http;

use crate::fixture;

#[test]
fn get_identity_then_token() -> Result<()> {
    fixture::RT.block_on(async {
        let fx = fixture::conformance().await?;

        let response = http::get(&fx.runtime, "/identity").await?;
        assert!(
            response.status().is_success(),
            "guest resolves an identity and obtains a token across the boundary: {:?}",
            response.body()
        );

        Ok(())
    })
}
