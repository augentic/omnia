use wasmtime::component::{Access, Accessor, Resource};

pub use crate::host::generated::omnia::websocket::types::{
    Error, Host, HostClient, HostClientWithStore, HostEvent, HostEventWithStore, SocketAddr,
};
use crate::host::resource::{ClientProxy, Event};
use crate::host::{Result, WasiWebSocket, WasiWebSocketCtxView};

impl<T> HostClientWithStore<T> for WasiWebSocket {
    async fn connect(accessor: &Accessor<T, Self>, _name: String) -> Result<Resource<ClientProxy>> {
        let socket = accessor.with(|mut store| store.get().ctx.connect()).await?;
        let proxy = ClientProxy(socket);
        Ok(accessor.with(|mut store| store.get().table.push(proxy))?)
    }

    fn disconnect(_: Access<'_, T, Self>, _: Resource<ClientProxy>) -> Result<()> {
        Ok(())
    }

    fn drop(mut accessor: Access<'_, T, Self>, rep: Resource<ClientProxy>) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl<T> HostEventWithStore<T> for WasiWebSocket {
    /// Create a new event with the given payload.
    fn new(mut host: Access<'_, T, Self>, data: Vec<u8>) -> wasmtime::Result<Resource<Event>> {
        Ok(host.get().table.push(Event::new(data))?)
    }

    /// The socket address this event was received from.
    fn socket_addr(
        mut host: Access<'_, T, Self>, self_: Resource<Event>,
    ) -> wasmtime::Result<Option<SocketAddr>> {
        let event = host.get().table.get(&self_)?;
        Ok(event.socket_addr.clone())
    }

    /// The event data.
    fn data(mut host: Access<'_, T, Self>, self_: Resource<Event>) -> wasmtime::Result<Vec<u8>> {
        let event = host.get().table.get(&self_)?;
        Ok(event.data.clone())
    }

    fn drop(mut accessor: Access<'_, T, Self>, rep: Resource<Event>) -> wasmtime::Result<()> {
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
    accessor: &Accessor<T, WasiWebSocket>, self_: &Resource<Event>,
) -> Result<Event> {
    accessor.with(|mut store| {
        let event = store.get().table.get(self_)?;
        Ok::<_, Error>(event.clone())
    })
}
