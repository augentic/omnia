use wasmtime::component::{Accessor, Resource};

use crate::host::generated::wasi::websocket::client::{Host, HostWithStore};
use crate::host::generated::wasi::websocket::types::Group;
use crate::host::resource::{EventProxy, SocketProxy};
use crate::host::types_impl::{get_event, get_socket};
use crate::host::{Result, WasiWebSocket, WasiWebSocketCtxView};

impl HostWithStore for WasiWebSocket {
    async fn send<T>(
        accessor: &Accessor<T, Self>, s: Resource<SocketProxy>, event: Resource<EventProxy>,
        group: Option<Vec<Group>>,
    ) -> Result<()> {
        let socket = get_socket(accessor, &s)?;
        let evt = get_event(accessor, &event)?;
        socket.send(evt, group).await?;

        Ok(())
    }
}

impl Host for WasiWebSocketCtxView<'_> {}
