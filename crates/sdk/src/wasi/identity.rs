//! [`Identity`] over `omnia:identity`.

use std::time::Duration;

use omnia_wasi_identity::credentials;

use crate::capability::{Error, ErrorKind, Result};
use crate::identity::{AccessToken, DEFAULT};
use crate::{Identity, WasiProvider};

impl Identity for WasiProvider {
    type Named = Handle;

    async fn token(&self, scopes: &[&str]) -> Result<AccessToken> {
        Handle::new(DEFAULT).token(scopes).await
    }

    fn named(&self, name: &str) -> Handle {
        Handle::new(name)
    }
}

/// An [`Identity`] handle on one identity of the WASI host.
///
/// [`Identity::named`] on [`WasiProvider`] returns one.
#[derive(Clone, Debug)]
pub struct Handle {
    name: String,
}

impl Handle {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
        }
    }
}

impl Identity for Handle {
    type Named = Self;

    async fn token(&self, scopes: &[&str]) -> Result<AccessToken> {
        let identity = credentials::get_identity(self.name.clone()).await?;
        let scopes = scopes.iter().map(|&scope| scope.to_owned()).collect();
        let token = identity.get_token(scopes).await?;
        Ok(AccessToken::new(token.token, Duration::from_secs(token.expires_in)))
    }

    fn named(&self, name: &str) -> Self {
        Self::new(name)
    }
}

impl From<credentials::Error> for Error {
    fn from(err: credentials::Error) -> Self {
        match err {
            credentials::Error::NoSuchIdentity => {
                Self::new(ErrorKind::NotFound, "no such identity")
            }
            credentials::Error::InternalFailure(message) => Self::new(ErrorKind::Other, message),
        }
    }
}
