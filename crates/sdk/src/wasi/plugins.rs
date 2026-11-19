//! [`Plugins`] over `omnia:plugins`.

use omnia_plugin::omnia::{loader, types};

use crate::plugins::{Digest, Error, Location, Plugin};
use crate::{Plugins, WasiProvider};

impl Plugins for WasiProvider {
    async fn load(&self, from: &Location, digest: Option<&Digest>) -> Result<Plugin, Error> {
        let when = digest.map(ToString::to_string);
        match loader::load(types::Source::from(from), when).await? {
            Ok(loaded) => {
                let digest = loaded.digest.parse().map_err(|error: Error| {
                    Error::Internal(format!("host reported a malformed digest: {error}"))
                })?;
                Ok(Plugin::new(loaded.name, digest, loaded.exports))
            }
            Err(conflict) => Err(Error::Refused(conflicted(from, digest, &conflict))),
        }
    }
}

// the bytes found and what they missed: the load's pin, or the bytes already
// active under the name
fn conflicted(from: &Location, pin: Option<&Digest>, conflict: &types::Conflict) -> String {
    let found = &conflict.found;
    match (pin, &conflict.current) {
        (Some(pin), _) if pin.as_str() != found => {
            format!("`{from}` resolved to {found}, not the pinned {pin}")
        }
        (_, Some(current)) => {
            format!("`{}` is loaded from {}, not {found}", current.name, current.digest)
        }
        (_, None) => format!("`{}` was active under bytes other than {found}", from.name()),
    }
}

impl From<&Location> for types::Source {
    fn from(location: &Location) -> Self {
        match location {
            Location::Declared(name) => Self::Declared(name.clone()),
            Location::Path(path) => Self::Path(path.clone()),
            Location::Registry { package, endpoint } => Self::Release(types::Release {
                reference: package.clone(),
                registry: endpoint.clone(),
            }),
        }
    }
}

// a retry cannot mend what the request or the grant got wrong, so those
// cases refuse
impl From<types::Error> for Error {
    fn from(error: types::Error) -> Self {
        match error {
            types::Error::NotFound(detail)
            | types::Error::Denied(detail)
            | types::Error::Invalid(detail) => Self::Refused(detail),
            types::Error::Unavailable(detail) => Self::Unavailable(detail),
            types::Error::Unsupported(detail) | types::Error::Other(detail) => {
                Self::Internal(detail)
            }
        }
    }
}
