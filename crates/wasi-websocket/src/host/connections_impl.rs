//! The `omnia:websocket/connections` host bindings.

use omnia_core::HostView;
use wasmtime::component::{Accessor, Resource};

use crate::host::generated::omnia::websocket::connections::{Host, HostWithStore};
use crate::host::generated::omnia::websocket::types::{self, ConnectionId};
use crate::host::resource::{ClientProxy, CloseFrame, Event};
use crate::host::types_impl::get_client;
use crate::host::{Result, WasiWebSocket, WebSocketClient};

impl<T> HostWithStore<T> for WasiWebSocket {
    async fn send(
        accessor: &Accessor<T, Self>, c: Resource<ClientProxy>, connection: ConnectionId,
        message: types::Message,
    ) -> Result<()> {
        let client = get_client(accessor, &c)?;
        client.send(Event::from(message), Some(vec![connection])).await?;

        Ok(())
    }

    async fn close(
        accessor: &Accessor<T, Self>, c: Resource<ClientProxy>, connection: ConnectionId,
        frame: Option<types::CloseFrame>,
    ) -> Result<()> {
        let frame = frame.map(CloseFrame::try_from).transpose()?;
        let client = get_client(accessor, &c)?;
        client.close(connection, frame).await?;

        Ok(())
    }

    async fn exists(
        accessor: &Accessor<T, Self>, c: Resource<ClientProxy>, connection: ConnectionId,
    ) -> Result<bool> {
        let client = get_client(accessor, &c)?;
        Ok(client.exists(connection).await?)
    }
}

impl Host for HostView<'_, dyn WebSocketClient> {}
