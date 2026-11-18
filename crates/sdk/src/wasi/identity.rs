//! [`Identity`] over `omnia:identity`.

use std::borrow::Cow;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use omnia_wasi_identity::omnia::{credentials, types};

use crate::identity::{AccessToken, DEFAULT};
use crate::{Error, HasIdentity, Identity, Result, WasiProvider};

static DEFAULT_HANDLE: Handle = Handle {
    name: Cow::Borrowed(DEFAULT),
};

impl HasIdentity for WasiProvider {
    type Inner = Handle;

    fn identity(&self) -> &Handle {
        &DEFAULT_HANDLE
    }
}

/// An [`Identity`] handle on one identity of the WASI host.
///
/// [`Identity::named`] on [`WasiProvider`] returns one.
#[derive(Clone, Debug)]
pub struct Handle {
    name: Cow<'static, str>,
}

impl Identity for Handle {
    type Named = Self;

    async fn token(&self, scopes: &[&str]) -> Result<AccessToken> {
        let identity = credentials::open(self.name.to_string()).await?;
        let scopes = scopes.iter().map(|&scope| scope.to_owned()).collect();
        let token = identity.get_token(scopes).await?;
        Ok(AccessToken::new(token.token, time_left(token.expires_at)))
    }

    fn named(&self, name: &str) -> Self {
        Self {
            name: Cow::Owned(name.to_owned()),
        }
    }
}

// the time left before `expires_at`, an instant in milliseconds since the Unix
// epoch, and zero once it has passed
fn time_left(expires_at: u64) -> Duration {
    UNIX_EPOCH.checked_add(Duration::from_millis(expires_at)).map_or(Duration::MAX, |expiry| {
        expiry.duration_since(SystemTime::now()).unwrap_or_default()
    })
}

impl From<types::Error> for Error {
    fn from(err: types::Error) -> Self {
        match err {
            types::Error::NotFound(message) => Self::server_error("not-found", message),
            types::Error::Denied(message) => Self::server_error("denied", message),
            types::Error::Invalid(message) => Self::server_error("invalid", message),
            types::Error::Unsupported(message) => Self::server_error("unsupported", message),
            types::Error::Unavailable(message) => Self::bad_gateway("unavailable", message),
            types::Error::Other(message) => Self::server_error("other", message),
        }
    }
}
