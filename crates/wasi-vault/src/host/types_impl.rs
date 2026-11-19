use omnia_core::HostView;

use crate::host::generated::omnia::types::types as omnia_types;
use crate::host::generated::omnia::vault::types;
use crate::host::{Error, VaultClient};

impl types::Host for HostView<'_, dyn VaultClient> {}

impl omnia_types::Host for HostView<'_, dyn VaultClient> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}
