# HTTP Cache Example

Demonstrates caching outbound HTTP responses with `omnia_sdk::http::cache`.

This example shows how to:

- Fetch through `HttpCache` at the call site
- Store responses in the `default` `omnia:keyvalue` bucket
- Drive the cache with `Cache-Control: max-age` and `If-None-Match`

The guest serves `GET /cached`, which fetches `{UPSTREAM_URL}/resource`
through the cache. The first request reaches the origin, and the response is
stored for 300 seconds. Every request after that is answered from the bucket.

## Quick Start

`UPSTREAM_URL` is read from the runtime's environment through `omnia:config`,
which serves it because the runtime compiles it into `ConfigDefault`'s keys.
Any origin that answers `GET /resource` with a 2xx status will do:

```bash
UPSTREAM_URL=https://httpbin.org/anything make example http-cache
```

Or, more manually, for debugging:

```bash
# build the guest
cargo build --example http-cache-wasm --target wasm32-wasip3

# run the host
export UPSTREAM_URL=https://httpbin.org/anything
export RUST_LOG="info,omnia_wasi_http=debug,omnia_sdk::http::cache=debug"
cargo run --example http-cache -- run ./target/wasm32-wasip3/debug/examples/http_cache_wasm.wasm
```

## Test

```bash
# first call: origin hit, response stored under the etag's store_key
curl -i http://localhost:8080/cached

# second call: served from the bucket; no upstream request is logged
curl -i http://localhost:8080/cached
```

Both responses carry `ETag: "v1"`, the request's entity-tag the cache stamps.
With the `RUST_LOG` above, the second call logs `cache hit` and
`omnia_wasi_http` logs only one outbound request.

The header contract (the directives, the strong single entity-tag, and what
is and is not stored) is documented on the
[`omnia_sdk::http::cache`](../../crates/sdk/src/http/cache.rs) module.
