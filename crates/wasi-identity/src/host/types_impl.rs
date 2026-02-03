use crate::host::WasiIdentityCtxView;
use crate::host::generated::wasi::identity::types;

impl types::Host for WasiIdentityCtxView<'_> {
    fn convert_error(&mut self, err: types::Error) -> wasmtime::Result<types::Error> {
        tracing::error!("{err}");
        Ok(err)
    }
}
