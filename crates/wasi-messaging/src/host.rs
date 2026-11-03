mod producer_impl;
mod request_reply_impl;
mod resource;
mod server;
mod types_impl;

mod generated {
    #![allow(missing_docs, reason = "wasmtime bindgen output")]

    pub use wasi::messaging::types::Error;

    pub use crate::host::resource::{ClientProxy, Message, RequestOptions};

    wasmtime::component::bindgen!({
        world: "messaging-request-reply",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        exports: {
            default: store | tracing | trappable,
        },
        with: {
            "wasi:messaging/request-reply.request-options": RequestOptions,
            "wasi:messaging/types.client": ClientProxy,
            "wasi:messaging/types.message": Message,
        },
        trappable_error_type: {
            "wasi:messaging/types.error" => Error,
        },
    });
}

use std::fmt::Debug;

pub use omnia_backend::messaging::{
    Endpoint, Error, HandleError, Handler, Message, MessagingClient, MessagingDefault, Metadata,
    Reply, RequestOptions, Subscriptions, dispatch,
};
pub use omnia_core::FutureResult;
use omnia_core::{Host, Runtime, Server, StoreCtx, StoreView};
use wasmtime::component::{HasData, Linker};

pub use self::generated::MessagingRequestReply;
use self::generated::wasi::messaging::types::Error as WitError;
use self::generated::wasi::messaging::{producer, request_reply, types};
pub use self::resource::ClientProxy;
pub use self::server::MessagingHandler;

/// Result type for messaging operations.
pub type Result<T> = std::result::Result<T, WitError>;

/// Host-side service for `wasi:messaging`.
#[derive(Debug)]
pub struct WasiMessaging;

impl HasData for WasiMessaging {
    type Data<'a> = WasiMessagingView<'a>;
}

impl<T> Host<T> for WasiMessaging
where
    T: StoreView<Self> + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        producer::add_to_linker::<_, Self>(linker, T::view)?;
        request_reply::add_to_linker::<_, Self>(linker, T::view)?;
        Ok(types::add_to_linker::<_, Self>(linker, T::view)?)
    }
}

impl<B> Server<B> for WasiMessaging
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: StoreView<Self>,
{
    async fn run(&self, state: &Runtime<B>) -> anyhow::Result<()> {
        server::run(state).await
    }
}

omnia_core::host_error!(WitError, Other);
omnia_core::wasi_view!(Messaging);
