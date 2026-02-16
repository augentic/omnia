use wasmtime::component::{Access, Accessor, Resource};

pub use crate::host::generated::wasi::websocket::types::{
    Error, Group, Host, HostEvent, HostEventWithStore, HostSocket, HostSocketWithStore,
};
use crate::host::resource::{EventProxy, SocketProxy};
use crate::host::{Result, WasiWebSocket, WasiWebSocketCtxView};

impl HostSocketWithStore for WasiWebSocket {
    async fn connect<T>(
        accessor: &Accessor<T, Self>, _name: String,
    ) -> Result<Resource<SocketProxy>> {
        let socket = accessor.with(|mut store| store.get().ctx.connect()).await?;
        let proxy = SocketProxy(socket);
        Ok(accessor.with(|mut store| store.get().table.push(proxy))?)
    }

    fn disconnect<T>(_: Access<'_, T, Self>, _: Resource<SocketProxy>) -> Result<()> {
        Ok(())
    }

    fn drop<T>(
        mut accessor: Access<'_, T, Self>, rep: Resource<SocketProxy>,
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

    /// The group this event was received on, if any.
    fn group<T>(
        mut host: Access<'_, T, Self>, self_: Resource<EventProxy>,
    ) -> wasmtime::Result<Option<Group>> {
        let event = host.get().table.get(&self_)?;
        Ok(event.group())
    }

    /// The event data.
    fn data<T>(
        mut host: Access<'_, T, Self>, self_: Resource<EventProxy>,
    ) -> wasmtime::Result<Vec<u8>> {
        let event = host.get().table.get(&self_)?;
        Ok(event.data())
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
impl HostSocket for WasiWebSocketCtxView<'_> {}
impl HostEvent for WasiWebSocketCtxView<'_> {}

pub fn get_socket<T>(
    accessor: &Accessor<T, WasiWebSocket>, self_: &Resource<SocketProxy>,
) -> Result<SocketProxy> {
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
