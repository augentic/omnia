//! [`SecretStore`] over `omnia:vault`.

use std::borrow::Cow;

use bytes::Bytes;
use omnia_wasi_vault::omnia::locker::Locker;
use omnia_wasi_vault::omnia::{store, types};

use crate::vault::DEFAULT;
use crate::{Error, HasSecretStore, Page, Result, SecretStore, WasiProvider};

static DEFAULT_HANDLE: Handle = Handle {
    name: Cow::Borrowed(DEFAULT),
};

impl HasSecretStore for WasiProvider {
    type Inner = Handle;

    fn secret_store(&self) -> &Handle {
        &DEFAULT_HANDLE
    }
}

/// A [`SecretStore`] handle on one locker of the WASI host.
///
/// [`SecretStore::named`] on [`WasiProvider`] returns one.
#[derive(Clone, Debug)]
pub struct Handle {
    name: Cow<'static, str>,
}

impl Handle {
    async fn open(&self) -> Result<Locker> {
        Ok(store::open(self.name.to_string()).await?)
    }
}

impl SecretStore for Handle {
    type Named = Self;

    async fn get(&self, key: &str) -> Result<Option<Bytes>> {
        Ok(self.open().await?.get(key.to_owned()).await?.map(Bytes::from))
    }

    async fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        Ok(self.open().await?.set(key.to_owned(), value.to_vec()).await?)
    }

    async fn delete(&self, key: &str) -> Result<()> {
        Ok(self.open().await?.delete(key.to_owned()).await?)
    }

    async fn keys(&self, after: Option<&str>) -> Result<Page<String>> {
        let locker = self.open().await?;
        let names = locker.list_secrets(String::new(), after.map(str::to_owned), None).await?;
        Ok(Page::new(names.secrets, names.next))
    }

    fn named(&self, name: &str) -> Self {
        Self {
            name: Cow::Owned(name.to_owned()),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        Ok(self.open().await?.exists(key.to_owned()).await?)
    }
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
