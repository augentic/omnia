use wasmtime::component::{Accessor, Resource};

use crate::host::generated::wasi::websocket::client::{Host, HostWithStore};
use crate::host::generated::wasi::websocket::types::SocketAddr;
use crate::host::resource::{ClientProxy, EventProxy};
use crate::host::types_impl::{get_client, get_event};
use crate::host::{Result, WasiWebSocket, WasiWebSocketCtxView};

impl HostWithStore for WasiWebSocket {
    async fn send<T>(
        accessor: &Accessor<T, Self>, s: Resource<ClientProxy>, event: Resource<EventProxy>,
        sockets: Option<Vec<SocketAddr>>,
    ) -> Result<()> {
        let client = get_client(accessor, &s)?;
        let evt = get_event(accessor, &event)?;
        client.send(evt, sockets).await?;

        Ok(())
    }
}

impl Host for WasiWebSocketCtxView<'_> {}
