use omnia_core::HostView;

use crate::host::generated::omnia::config::types;
use crate::host::generated::omnia::types::types as omnia_types;
use crate::host::{ConfigClient, Error};

impl types::Host for HostView<'_, dyn ConfigClient> {}

impl omnia_types::Host for HostView<'_, dyn ConfigClient> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}
