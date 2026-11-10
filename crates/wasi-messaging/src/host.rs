mod default_impl;
mod producer_impl;
mod request_reply_impl;
mod resource;
mod server;
mod types_impl;

mod generated {
    #![expect(missing_docs, reason = "wasmtime bindgen output")]

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

pub use default_impl::MessagingDefault;
pub use omnia_backend::messaging::{
    Endpoint, Error, HandleError, Handler, Message, MessagingClient, Metadata, Reply,
    RequestOptions, Subscriptions, dispatch,
};
pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, Provides, Runtime, StoreCtx};
use wasmtime::component::Linker;

pub use self::generated::MessagingRequestReply;
use self::generated::wasi::messaging::types::Error as WitError;
use self::generated::wasi::messaging::{producer, request_reply, types};
pub use self::resource::ClientProxy;
pub use self::server::MessagingHandler;

/// Result type for messaging operations.
pub type Result<T> = std::result::Result<T, WitError>;

/// Host-side service for `wasi:messaging`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = MessagingClient)]
pub struct WasiMessaging;

omnia_backend::capability!(WasiMessaging, "wasi:messaging", MessagingClient);

impl<B: Provides<Self>> Host<B> for WasiMessaging {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        producer::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        request_reply::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }

    async fn run(runtime: &Runtime<B>) -> anyhow::Result<()> {
        server::run(runtime).await
    }
}

omnia_core::host_error!(WitError, Other);
