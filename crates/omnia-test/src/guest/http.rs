//! A URL-matched `HttpRequest` recording every request.

use std::any::Any;
use std::error::Error;
use std::fmt;
use std::future::{Future, poll_fn};
use std::sync::{Arc, Mutex};
use std::task::{Poll, ready};

use anyhow::Result;
use bytes::Bytes;
use http::{HeaderMap, Method, Request, Response, StatusCode, Uri};
use http_body::{Body, Frame};
use omnia_guest::HttpRequest;

type BodyPredicate = Arc<dyn Fn(&[u8]) -> bool + Send + Sync>;

/// One request the code under test sent, with its body collected.
#[derive(Clone, Debug)]
pub struct Recorded {
    /// The request method.
    pub method: Method,
    /// The request URI.
    pub uri: Uri,
    /// The request headers.
    pub headers: HeaderMap,
    /// The collected request body.
    pub body: Vec<u8>,
}

impl Recorded {
    /// The body as UTF-8, lossily.
    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
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

#[derive(Default)]
struct Inner {
    routes: Mutex<Vec<Route>>,
    requests: Mutex<Vec<Recorded>>,
}

/// Outbound HTTP answered by the first route matching method, URL, and
/// (optionally) body; every request is recorded and an unmatched one
/// panics naming its method and URL.
///
/// ```
/// use bytes::Bytes;
/// use http::{Method, Request, Response};
/// use http_body_util::Full;
/// use omnia_guest::HttpRequest as _;
/// use omnia_test::guest::MatchedHttp;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let http = MatchedHttp::default().on(
///     Method::GET,
///     "https://api.test/ping",
///     Response::new(Bytes::from_static(b"pong")),
/// );
/// let request = Request::get("https://api.test/ping").body(Full::<Bytes>::default()).unwrap();
/// assert_eq!(http.fetch(request).await.unwrap().into_body(), "pong");
/// assert_eq!(http.requests()[0].uri.to_string(), "https://api.test/ping");
/// # });
/// ```
#[derive(Clone, Default)]
pub struct MatchedHttp {
    inner: Arc<Inner>,
}

impl fmt::Debug for MatchedHttp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MatchedHttp")
            .field("routes", &self.inner.routes.lock().map_or(0, |r| r.len()))
            .field("requests", &self.requests())
            .finish()
    }
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

impl HttpRequest for MatchedHttp {
    /// # Panics
    ///
    /// Panics when no route matches the request.
    fn fetch<T>(&self, request: Request<T>) -> impl Future<Output = Result<Response<Bytes>>> + Send
    where
        T: Body + Any + Send,
        T::Data: Into<Vec<u8>>,
        T::Error: Into<Box<dyn Error + Send + Sync + 'static>>,
    {
        let (parts, body) = request.into_parts();
        let this = self.clone();
        async move {
            let body = collect(body).await.map_err(anyhow::Error::from_boxed)?;
            Ok(this.answer(Recorded {
                method: parts.method,
                uri: parts.uri,
                headers: parts.headers,
                body,
            }))
        }
    }
}

// Drains `body` into bytes frame by frame. `BodyExt::collect` would hold
// `T::Data` across an await, and the trait does not promise that type is
// `Send`; converting each frame inside `poll` keeps the future `Send`.
fn collect<T>(body: T) -> impl Future<Output = Result<Vec<u8>, Box<dyn Error + Send + Sync>>> + Send
where
    T: Body + Send,
    T::Data: Into<Vec<u8>>,
    T::Error: Into<Box<dyn Error + Send + Sync + 'static>>,
{
    let mut body = Box::pin(body);
    let mut bytes = Vec::new();
    poll_fn(move |cx| {
        loop {
            match ready!(body.as_mut().poll_frame(cx)) {
                Some(Ok(frame)) => {
                    if let Ok(data) = Frame::into_data(frame) {
                        bytes.extend(data.into());
                    }
                }
                Some(Err(error)) => return Poll::Ready(Err(error.into())),
                None => return Poll::Ready(Ok(std::mem::take(&mut bytes))),
            }
        }
    })
}
