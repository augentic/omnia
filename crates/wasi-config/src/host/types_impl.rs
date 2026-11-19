use omnia_core::HostView;

use crate::host::ConfigClient;
use crate::host::generated::omnia::config::types;

impl types::Host for HostView<'_, dyn ConfigClient> {
    fn convert_error(&mut self, err: types::Error) -> wasmtime::Result<types::Error> {
        Ok(err)
    }
}
