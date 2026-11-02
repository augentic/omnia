use crate::host::WasiIdentityView;
use crate::host::generated::omnia::identity::types;

impl types::Host for WasiIdentityView<'_> {
    fn convert_error(&mut self, err: types::Error) -> wasmtime::Result<types::Error> {
        tracing::error!("{err}");
        Ok(err)
    }
}
