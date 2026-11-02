//! # WASI HTTP Host
//!
//! This module implements a host-side service for `wasi:http`.

mod server;

use std::fmt::Debug;
use std::future::Future;

use anyhow::Result;
use http_body_util::BodyExt as _;
use omnia_backend::http::HttpDefault as BackendHttpDefault;
pub use omnia_backend::http::{
    Body, ConnectOptions, Error as ClientError, HttpClient, RequestOptions as ClientRequestOptions,
    client_cert,
};
use omnia_core::{Connect, Host, HostCtx, Runtime, Server, StoreCtx, StoreView};
pub use server::{HttpHandler, OutgoingBody};
use wasmtime::component::{HasData, Linker, ResourceTable};
use wasmtime_wasi_http::p3::bindings::http::{client, types};
use wasmtime_wasi_http::{Error as WasiError, RequestOptions, WasiBody, WasiHttpHooks};
pub use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView};

/// Default [`WasiHttp`] slot wrapping [`omnia_backend::http::HttpDefault`].
pub type HttpDefault = HttpSlot<BackendHttpDefault>;

/// Host-side service for `wasi:http`.
#[derive(Debug)]
pub struct WasiHttp;

impl HasData for WasiHttp {
    type Data<'a> = WasiHttpCtxView<'a>;
}

impl HostCtx for WasiHttp {
    type Borrow<'a> = &'a mut dyn WasiHttpHooks;

    fn view<'a>(borrow: Self::Borrow<'a>, table: &'a mut ResourceTable) -> WasiHttpCtxView<'a> {
        WasiHttpCtxView {
            hooks: borrow,
            ctx: default_ctx(),
            table,
        }
    }
}

// `WasiHttpCtx` only holds the constant `field_size_limit` read by shared ref.
fn default_ctx() -> &'static mut WasiHttpCtx {
    Box::leak(Box::new(WasiHttpCtx::new()))
}

impl<T> Host<T> for WasiHttp
where
    T: StoreView<Self> + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> Result<()> {
        client::add_to_linker::<_, wasmtime_wasi_http::WasiHttp>(linker, T::view)?;
        types::add_to_linker::<_, wasmtime_wasi_http::WasiHttp>(linker, T::view)?;
        Ok(())
    }
}

impl<B> Server<B> for WasiHttp
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: StoreView<Self>,
{
    async fn run(&self, state: &Runtime<B>) -> Result<()> {
        server::run(state).await
    }
}

/// Store-slot adapter wrapping an [`HttpClient`] so it implements [`WasiHttpHooks`].
#[derive(Clone, Debug)]
pub struct HttpSlot<C = BackendHttpDefault> {
    /// The underlying outbound HTTP client.
    pub client: C,
}

impl<C> HttpSlot<C> {
    /// Wrap an already-connected [`HttpClient`].
    #[must_use]
    pub const fn new(client: C) -> Self {
        Self { client }
    }
}

impl<C: Connect> Connect for HttpSlot<C> {
    type ConnectOptions = C::ConnectOptions;

    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        Ok(Self::new(C::connect_with(options).await?))
    }
}

impl<C: HttpClient> HttpClient for HttpSlot<C> {
    fn fetch(
        &self, request: http::Request<Body>, options: Option<ClientRequestOptions>,
    ) -> omnia_core::futures::future::BoxFuture<'static, Result<http::Response<Body>, ClientError>>
    {
        self.client.fetch(request, options)
    }
}

impl<C: HttpClient> WasiHttpHooks for HttpSlot<C> {
    fn set_host_header(&mut self) -> bool {
        false
    }

    fn send_request(
        &mut self, request: http::Request<WasiBody>, options: Option<RequestOptions>,
        fut: Box<dyn Future<Output = Result<(), WasiError>> + Send>,
    ) -> Box<
        dyn Future<
                Output = Result<
                    (
                        http::Response<WasiBody>,
                        Box<dyn Future<Output = Result<(), WasiError>> + Send>,
                    ),
                    WasiError,
                >,
            > + Send,
    > {
        let options = options.map(|o| ClientRequestOptions {
            connect_timeout: o.connect_timeout,
            first_byte_timeout: o.first_byte_timeout,
            between_bytes_timeout: o.between_bytes_timeout,
        });
        let request = request.map(|body| {
            body.map_err(|e| ClientError::InternalError(Some(e.to_string()))).boxed_unsync()
        });
        let pending = self.client.fetch(request, options);
        Box::new(async move {
            let response = pending.await.map_err(to_wasi_error)?;
            let response = response.map(|body| body.map_err(to_wasi_error).boxed_unsync());
            Ok((response, fut))
        })
    }
}

fn to_wasi_error(err: ClientError) -> WasiError {
    match err {
        ClientError::Connect(io) => WasiError::Connect(io),
        ClientError::Tls(io) => WasiError::Tls(io),
        ClientError::ConnectionRefused => WasiError::ConnectionRefused,
        ClientError::ConnectionTimeout => WasiError::ConnectionTimeout,
        ClientError::ConnectionReadTimeout => WasiError::ConnectionReadTimeout,
        ClientError::TlsCertificateError => WasiError::TlsCertificateError,
        ClientError::TlsAlertReceived {
            alert_id,
            alert_message,
        } => WasiError::TlsAlertReceived {
            alert_id,
            alert_message,
        },
        ClientError::HttpRequestUriInvalid => WasiError::HttpRequestUriInvalid,
        ClientError::HttpResponseIncomplete => WasiError::HttpResponseIncomplete,
        ClientError::HttpProtocolError => WasiError::HttpProtocolError,
        ClientError::InternalError(msg) => WasiError::InternalError(msg),
        _ => WasiError::InternalError(Some(err.to_string())),
    }
}
