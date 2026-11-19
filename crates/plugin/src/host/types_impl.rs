use crate::host::WasiPluginsView;
use crate::host::generated::omnia::plugins::types;

impl types::Host for WasiPluginsView {
    fn convert_error(&mut self, err: types::Error) -> wasmtime::Result<types::Error> {
        tracing::debug!("plugin load failed: {err}");
        Ok(err)
    }
}
