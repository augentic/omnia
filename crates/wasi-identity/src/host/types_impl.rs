use omnia_core::HostView;

use crate::host::IdentityClient;
use crate::host::generated::omnia::identity::types;

impl types::Host for HostView<'_, dyn IdentityClient> {
    fn convert_error(&mut self, err: types::Error) -> wasmtime::Result<types::Error> {
        tracing::error!("{err}");
        Ok(err)
    }
}
