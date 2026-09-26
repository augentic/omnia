# HTTP Proxy Example

Demonstrates an HTTP proxy: a guest that makes outbound requests through `wasi-http` and relays the origin's response.

This example shows how to:

- Make outgoing HTTP requests from within a WASI guest
- Relay an origin response (status, headers, body) back to the caller
- Present a client certificate on outbound requests (mTLS)

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
export RUST_LOG="info,omnia_wasi_http=debug,http_proxy=debug"
cargo run --example http-proxy -- run ./target/wasm32-wasip2/debug/examples/http_proxy_wasm.wasm
```

## Test

```bash
# GET from origin and return
curl http://localhost:8080/origin-sm
```

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

Before the bundle becomes a TLS identity the host validates its **first
certificate** — the identity presented to the server. It is rejected if its
Extended Key Usage excludes `clientAuth` (a server-only certificate), if it is
itself a CA certificate, if its Key Usage does not permit signing, or if it is
outside its validity window. A certificate with no Extended Key Usage
extension is accepted, per RFC 5280. Chain certificates following it
(intermediate and root CAs) are **not inspected**: a bundle of
`leaf + intermediate CA + root CA` passes; only a CA certificate placed
*first*, where the identity certificate belongs, fails.
