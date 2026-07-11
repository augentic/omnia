mod default_impl;
mod producer_impl;
mod request_reply_impl;
mod resource;
mod server;
mod types_impl;

mod generated {
    #![allow(missing_docs)]

    pub use wasi::messaging::types::Error;

    pub use crate::host::resource::{ClientProxy, Message, RequestOptions};

    wasmtime::component::bindgen!({
        world: "messaging-request-reply",
        path: "wit",
        imports: {
            // "wasi:messaging/types.[static]client.connect": store | tracing | trappable,
            default:  store | tracing | trappable,
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
        // include_generated_code_from_file: true,
    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use omnia::FutureResult;
use omnia::{Host, Runtime, Server, StoreCtx};
use wasmtime::component::{HasData, Linker};

pub use self::default_impl::MessagingDefault;
pub use self::generated::MessagingRequestReply;
pub use self::generated::wasi::messaging::types::Error;
use self::generated::wasi::messaging::{producer, request_reply, types};
pub use self::resource::*;

/// Result type for messaging operations.
pub type Result<T> = anyhow::Result<T, Error>;

/// Host-side service for `wasi:messaging`.
#[derive(Debug)]
pub struct WasiMessaging;

impl HasData for WasiMessaging {
    type Data<'a> = WasiMessagingCtxView<'a>;
}

impl<T> Host<T> for WasiMessaging
where
    T: WasiMessagingView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        producer::add_to_linker::<_, Self>(linker, T::messaging)?;
        request_reply::add_to_linker::<_, Self>(linker, T::messaging)?;
        Ok(types::add_to_linker::<_, Self>(linker, T::messaging)?)
    }
}

impl<B> Server<B> for WasiMessaging
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiMessagingView,
{
    const IS_SERVER: bool = true;

    async fn run(&self, state: &Runtime<B>) -> anyhow::Result<()> {
        server::run(state).await
    }
}

/// A trait which provides internal WASI Messaging context.
///
/// This is implemented by the resource-specific provider of messaging
/// functionality. For example, a NATS, or a Kafka broker.
///
/// Message construction and mutation live on the host's [`Message`] struct;
/// backends only translate at the [`Client`] seam.
pub trait WasiMessagingCtx: Debug + Send + Sync + 'static {
    /// Connect to the messaging system and return a client proxy.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails.
    fn connect(&self) -> FutureResult<Arc<dyn Client>>;
}

omnia::host_error!(Error, Other);
omnia::wasi_view!(Messaging);
