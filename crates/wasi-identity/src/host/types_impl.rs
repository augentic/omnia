use omnia_core::HostView;

use crate::host::generated::omnia::identity::types;
use crate::host::generated::omnia::types::types as omnia_types;
use crate::host::{Error, IdentityClient};

impl types::Host for HostView<'_, dyn IdentityClient> {}

impl omnia_types::Host for HostView<'_, dyn IdentityClient> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}
