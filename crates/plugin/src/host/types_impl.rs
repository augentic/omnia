use crate::host::WasiPluginsView;
use crate::host::generated::Error;
use crate::host::generated::omnia::plugins::types;

impl types::Host for WasiPluginsView {}

impl omnia_wasi_types::types::Host for WasiPluginsView {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        tracing::debug!("plugin load failed: {err}");
        Ok(err)
    }
}
