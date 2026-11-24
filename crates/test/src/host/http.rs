//! A URL-matched outbound `wasi:http` backend recording every request.

use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use bytes::Bytes;
use http::{HeaderMap, Method, Request, Response, StatusCode, Uri};
use http_body_util::{BodyExt as _, Full};
use omnia::Grants;
use omnia::wasmtime::component::ResourceTable;
use omnia_wasi_http::{
    ErrorCode, HttpBorrow, RequestOptions, WasiBody, WasiHttpCtx, WasiHttpCtxView, WasiHttpHooks,
};

/// One request a guest sent, with its body.
#[derive(Clone, Debug)]
pub struct Recorded {
    /// The request method.
    pub method: Method,
    /// The request URI.
    pub uri: Uri,
    /// The request headers.
    pub headers: HeaderMap,
    /// The request body.
    pub body: Bytes,
}

impl Recorded {
    /// The body as UTF-8, lossily.
    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Outbound `wasi:http` answered by the first route matching method, URL,
/// and optionally body.
///
/// A request to a host the guest's grants do not reach fails with
/// [`HttpRequestDenied`](ErrorCode::HttpRequestDenied), as it does over
/// [`HttpDefault`](omnia_wasi_http::HttpDefault), and is not recorded. Every
/// other request is recorded, matched or not. One no route matches fails the
/// guest's send with an [`InternalError`](ErrorCode::InternalError) naming
/// its method and URL, which the guest sees as a transport failure. Clones
/// share one set of routes and one record.
///
/// [`Backends::http`](super::Backends::http) serves it to a deployment.
#[derive(Clone, Default)]
pub struct MatchedHttp {
    hooks: Hooks,
    ctx: WasiHttpCtx,
}

impl MatchedHttp {
    /// Answers `method url` with `response`.
    #[must_use]
    pub fn on(self, method: Method, url: impl Into<String>, response: Response<Bytes>) -> Self {
        self.route(method, url.into(), None, response)
    }

    /// Answers `method url` with `response` when the request body satisfies
    /// `body`.
    #[must_use]
    pub fn on_matching(
        self, method: Method, url: impl Into<String>,
        body: impl Fn(&[u8]) -> bool + Send + Sync + 'static, response: Response<Bytes>,
    ) -> Self {
        self.route(method, url.into(), Some(Arc::new(body)), response)
    }

    /// Every request sent, in the order the host received them.
    #[must_use]
    pub fn requests(&self) -> Vec<Recorded> {
        self.hooks.inner.requests.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn route(
        self, method: Method, url: String, body: Option<BodyPredicate>, response: Response<Bytes>,
    ) -> Self {
        let (parts, payload) = response.into_parts();
        self.hooks.inner.routes.lock().unwrap_or_else(PoisonError::into_inner).push(Route {
            method,
            url,
            body,
            status: parts.status,
            headers: parts.headers,
            payload,
        });
        self
    }
}

impl HttpBorrow for MatchedHttp {
    fn as_view<'a>(
        &'a mut self, table: &'a mut ResourceTable, grants: &'a Arc<Grants>,
    ) -> WasiHttpCtxView<'a> {
        self.hooks.grants = Some(Arc::clone(grants));
        WasiHttpCtxView {
            hooks: &mut self.hooks,
            ctx: &mut self.ctx,
            table,
        }
    }
}

impl fmt::Debug for MatchedHttp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let routes = self.hooks.inner.routes.lock().map_or(0, |routes| routes.len());
        f.debug_struct("MatchedHttp")
            .field("routes", &routes)
            .field("requests", &self.requests())
            .finish_non_exhaustive()
    }
}

type BodyPredicate = Arc<dyn Fn(&[u8]) -> bool + Send + Sync>;
type Settled = Box<dyn Future<Output = Result<(), ErrorCode>> + Send>;

// A clone serves one store, so `grants` are one guest's, and none before the
// store's first view.
#[derive(Clone, Default)]
struct Hooks {
    inner: Arc<Inner>,
    grants: Option<Arc<Grants>>,
}

impl WasiHttpHooks for Hooks {
    fn send_request(
        &mut self, request: Request<WasiBody>, _options: Option<RequestOptions>, settled: Settled,
    ) -> Box<dyn Future<Output = Result<(Response<WasiBody>, Settled), ErrorCode>> + Send> {
        let inner = Arc::clone(&self.inner);
        let denied = request.uri().host().is_some_and(|host| {
            self.grants.as_ref().is_some_and(|grants| !grants.allows_host(host))
        });
        Box::new(async move {
            if denied {
                return Err(ErrorCode::HttpRequestDenied);
            }
            let (parts, body) = request.into_parts();
            let recorded = Recorded {
                method: parts.method,
                uri: parts.uri,
                headers: parts.headers,
                body: body.collect().await?.to_bytes(),
            };
            let response = inner.answer(&recorded);
            let unmatched = format!("no route matches {} {}", recorded.method, recorded.uri);
            inner.requests.lock().unwrap_or_else(PoisonError::into_inner).push(recorded);

            let response = response.ok_or(ErrorCode::InternalError(Some(unmatched)))?;
            let response =
                response.map(|body| Full::new(body).map_err(|never| match never {}).boxed_unsync());
            Ok((response, settled))
        })
    }
}

#[derive(Default)]
struct Inner {
    routes: Mutex<Vec<Route>>,
    requests: Mutex<Vec<Recorded>>,
}

impl Inner {
    fn answer(&self, recorded: &Recorded) -> Option<Response<Bytes>> {
        let routes = self.routes.lock().unwrap_or_else(PoisonError::into_inner);
        routes.iter().find(|route| route.matches(recorded)).map(Route::response)
    }
}

struct Route {
    method: Method,
    url: String,
    body: Option<BodyPredicate>,
    status: StatusCode,
    headers: HeaderMap,
    payload: Bytes,
}

impl Route {
    fn matches(&self, recorded: &Recorded) -> bool {
        self.method == recorded.method
            && self.url == recorded.uri.to_string()
            && self.body.as_ref().is_none_or(|matches| matches(&recorded.body))
    }

    fn response(&self) -> Response<Bytes> {
        let mut response = Response::new(self.payload.clone());
        *response.status_mut() = self.status;
        *response.headers_mut() = self.headers.clone();
        response
    }
}
