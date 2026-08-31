# HTTP Proxy Example

Demonstrates an HTTP proxy using `wasi-http` with a `wasi-keyvalue` caching layer.

This example shows how to:

- Make outgoing HTTP requests from within a WASI guest
- Implement response caching with `Cache-Control` headers
- Use ETags for cache validation

## Quick Start

```bash
make build http-proxy
make run http-proxy
```

Or, more manually, for debugging:

```bash
# build the guest
cargo build --example http-proxy-wasm --target wasm32-wasip2

# run the host
export RUST_LOG="info,opentelemetry_sdk=off,omnia_wasi_http=debug,http_proxy=debug"
cargo run --example http-proxy -- run ./target/wasm32-wasip2/debug/examples/http_proxy_wasm.wasm
```

## Test

```bash
# GET with cached response (2nd+ requests)
curl http://localhost:8080/cache

# GET from origin and return
curl http://localhost:8080/origin-sm

# POST to origin and cache the (large) response
curl --header 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080/origin-xl
```

## Implementing Caching

Use the [Cache-Control] header to influence the use of a pass-through cache. The following directives are currently supported:

- `no-cache` - make a request to the resource and then cache the result for future requests. Usually used alongside `max-age` for key-value stores that support ttl.

- `no-store` - make a request to the resource and do not update the cache. This has the same effect as leaving out the `Cache-Control` header altogether. No other directive can be used with this one otherwise an error will be returned.

- `max-age=n` - try the cache first and return the result if it exists. If the record doesn't exist, go to the resource then cache the result with an expiry of now plus *n* seconds (for key-value stores that support ttl).

Multiple directives can be combined in a comma-delimited list:

```http
Cache-Control: max-age=86400,forward=https://example.com/api/v1/records/2934875
```

> [!WARNING] Currently, the [Cache-Control] header requires a corresponding [If-None-Match] header with a single `<etag_value>` to use as the cache key.

In the example guest an HTTP POST will cause an error: the [If-None-Match] header has been omitted to demonstrate that the caching implementation requires the guest to set this header alongside the [Cache-Control] header.

## Client Certificates (mTLS)

The default `wasi-http` host supports mutual TLS on outbound requests. Set a
`Client-Cert` header containing the base64-encoded PEM bundle (client
certificate followed by its private key); the host strips the header before
sending, decodes it into a TLS identity, and issues the request through a
one-off client that presents the certificate. The header never reaches the
origin server.

```rust
use base64ct::{Base64, Encoding};

// PEM bundle: certificate first, then the private key.
let pem = std::fs::read("client-bundle.pem")?; // or fetch from wasi:vault
let request = http::Request::builder()
    .method(Method::GET)
    .uri("https://mtls.example.com/resource")
    .header("Client-Cert", Base64::encode_string(&pem))
    .body(Empty::<Bytes>::new())?;

let response = omnia_wasi_http::handle(request).await?;
```

Keep the private key out of guest source: load it from `wasi:vault` or
configuration at runtime. An invalid or malformed bundle fails the request
with an internal error before any connection is attempted.

[Cache-Control]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Cache-Control [If-None-Match]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/If-None-Match
