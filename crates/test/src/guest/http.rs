//! A URL-matched `HttpClient` recording every request.

use std::fmt;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http::{HeaderMap, Method, Request, Response, StatusCode, Uri};
use omnia_sdk::HttpClient;
use omnia_sdk::capability::Result;

/// One request the code under test sent, with its body.
#[derive(Clone, Debug)]
pub struct Recorded {
    /// The request method.
    pub method: Method,
    /// The request URI.
    pub uri: Uri,
    /// The request headers.
    pub headers: HeaderMap,
    /// The request body.
    pub body: Vec<u8>,
}

impl Recorded {
    /// The body as UTF-8, lossily.
    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Outbound HTTP answered by the first route matching method, URL, and
/// (optionally) body; every request is recorded and an unmatched one
/// panics naming its method and URL.
///
/// ```
/// use bytes::Bytes;
/// use http::{Method, Request, Response};
/// use omnia_sdk::HttpClient as _;
/// use omnia_test::guest::MatchedHttp;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let http = MatchedHttp::default().on(
///     Method::GET,
///     "https://api.test/ping",
///     Response::new(Bytes::from_static(b"pong")),
/// );
/// let request = Request::get("https://api.test/ping").body(Bytes::new()).unwrap();
/// assert_eq!(http.fetch(request).await.unwrap().into_body(), "pong");
/// assert_eq!(http.requests()[0].uri.to_string(), "https://api.test/ping");
/// # });
/// ```
#[derive(Clone, Default)]
pub struct MatchedHttp {
    inner: Arc<Inner>,
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

    fn route(
        self, method: Method, url: String, body: Option<BodyPredicate>, response: Response<Bytes>,
    ) -> Self {
        let (parts, payload) = response.into_parts();
        self.inner.routes.lock().expect("routes lock").push(Route {
            method,
            url,
            body,
            status: parts.status,
            headers: parts.headers,
            payload,
        });
        self
    }

    /// Every request sent, in call order.
    ///
    /// # Panics
    ///
    /// Panics if a lock is poisoned.
    #[must_use]
    pub fn requests(&self) -> Vec<Recorded> {
        self.inner.requests.lock().expect("requests lock").clone()
    }

    fn answer(&self, recorded: Recorded) -> Response<Bytes> {
        let response = self
            .inner
            .routes
            .lock()
            .expect("routes lock")
            .iter()
            .find(|route| route.matches(&recorded))
            .map(Route::response);
        let Some(response) = response else {
            panic!("no response scripted for {} {}", recorded.method, recorded.uri);
        };
        self.inner.requests.lock().expect("requests lock").push(recorded);
        response
    }
}

impl fmt::Debug for MatchedHttp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MatchedHttp")
            .field("routes", &self.inner.routes.lock().map_or(0, |r| r.len()))
            .field("requests", &self.requests())
            .finish()
    }
}

// panics when no route matches the request
impl HttpClient for MatchedHttp {
    fn fetch(
        &self, request: Request<Bytes>,
    ) -> impl Future<Output = Result<Response<Bytes>>> + Send {
        let (parts, body) = request.into_parts();
        ready(Ok(self.answer(Recorded {
            method: parts.method,
            uri: parts.uri,
            headers: parts.headers,
            body: body.to_vec(),
        })))
    }
}

#[derive(Default)]
struct Inner {
    routes: Mutex<Vec<Route>>,
    requests: Mutex<Vec<Recorded>>,
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

type BodyPredicate = Arc<dyn Fn(&[u8]) -> bool + Send + Sync>;
