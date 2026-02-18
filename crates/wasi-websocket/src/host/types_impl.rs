use wasmtime::component::{Access, Accessor, Resource};

pub use crate::host::generated::wasi::websocket::types::{
    Error, Host, HostClient, HostClientWithStore, HostEvent, HostEventWithStore, SocketAddr,
};
use crate::host::resource::{ClientProxy, EventProxy};
use crate::host::{Result, WasiWebSocket, WasiWebSocketCtxView};

impl HostClientWithStore for WasiWebSocket {
    async fn connect<T>(
        accessor: &Accessor<T, Self>, _name: String,
    ) -> Result<Resource<ClientProxy>> {
        let socket = accessor.with(|mut store| store.get().ctx.connect()).await?;
        let proxy = ClientProxy(socket);
        Ok(accessor.with(|mut store| store.get().table.push(proxy))?)
    }

    fn disconnect<T>(_: Access<'_, T, Self>, _: Resource<ClientProxy>) -> Result<()> {
        Ok(())
    }

    fn drop<T>(
        mut accessor: Access<'_, T, Self>, rep: Resource<ClientProxy>,
    ) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl HostEventWithStore for WasiWebSocket {
    /// Create a new event with the given payload.
    fn new<T>(
        mut host: Access<'_, T, Self>, data: Vec<u8>,
    ) -> wasmtime::Result<Resource<EventProxy>> {
        let event = host.get().ctx.new_event(data).map_err(wasmtime::Error::from_anyhow)?;
        let proxy = EventProxy(event);
        Ok(host.get().table.push(proxy)?)
    }

    /// The socket address this event was received from.
    fn socket_addr<T>(
        mut host: Access<'_, T, Self>, self_: Resource<EventProxy>,
    ) -> wasmtime::Result<Option<SocketAddr>> {
        let event = host.get().table.get(&self_)?;
        Ok(event.socket_addr().map(String::from))
    }

    /// The event data.
    fn data<T>(
        mut host: Access<'_, T, Self>, self_: Resource<EventProxy>,
    ) -> wasmtime::Result<Vec<u8>> {
        let event = host.get().table.get(&self_)?;
        Ok(event.data().to_vec())
    }

    fn drop<T>(
        mut accessor: Access<'_, T, Self>, rep: Resource<EventProxy>,
    ) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl Host for WasiWebSocketCtxView<'_> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}
impl HostClient for WasiWebSocketCtxView<'_> {}
impl HostEvent for WasiWebSocketCtxView<'_> {}

pub fn get_client<T>(
    accessor: &Accessor<T, WasiWebSocket>, self_: &Resource<ClientProxy>,
) -> Result<ClientProxy> {
    accessor.with(|mut store| {
        let socket = store.get().table.get(self_)?;
        Ok::<_, Error>(socket.clone())
    })
}

pub fn get_event<T>(
    accessor: &Accessor<T, WasiWebSocket>, self_: &Resource<EventProxy>,
) -> Result<EventProxy> {
    accessor.with(|mut store| {
        let event = store.get().table.get(self_)?;
        Ok::<_, Error>(event.clone())
    })
}
