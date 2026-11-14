//! [`SecretStore`] over `omnia:vault`.

use std::borrow::Cow;

use bytes::Bytes;
use omnia_wasi_vault::vault;

use crate::capability::{Error, ErrorKind, Page, Result};
use crate::vault::DEFAULT;
use crate::{HasSecretStore, SecretStore, WasiProvider};

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
    async fn open(&self) -> Result<vault::Locker> {
        Ok(vault::open(self.name.to_string()).await?)
    }
}

// `omnia:vault` lists a locker in one call, so `keys` answers one page in key
// order and resumes after the key it is given
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
        let mut keys = self.open().await?.list_ids().await?;
        keys.retain(|key| after.is_none_or(|after| key.as_str() > after));
        keys.sort_unstable();
        Ok(Page::new(keys, None))
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

impl From<vault::Error> for Error {
    fn from(err: vault::Error) -> Self {
        match err {
            vault::Error::NoSuchStore => Self::new(ErrorKind::NotFound, "no such locker"),
            vault::Error::AccessDenied => Self::new(ErrorKind::Denied, "access denied"),
            vault::Error::Other(message) => Self::new(ErrorKind::Other, message),
        }
    }
}
