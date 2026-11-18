use omnia_core::HostView;

use crate::host::VaultClient;
use crate::host::generated::omnia::vault::types;

impl types::Host for HostView<'_, dyn VaultClient> {
    fn convert_error(&mut self, err: types::Error) -> wasmtime::Result<types::Error> {
        Ok(err)
    }
}
