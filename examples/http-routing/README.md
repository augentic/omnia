# Multi-guest HTTP routing

Two HTTP guests (`a` and `b`) behind path prefixes, wired through a deployment manifest. The host runs a single HTTP server; the manifest's `[[route.http]]` table selects the guest per request by longest-prefix match.

Because two guests export `wasi:http/incoming-handler`, the routes are **required** — without them startup fails with an ambiguity error. With a single guest (as in the other examples) routing falls back to a catch-all and no manifest is needed.

## Build the guests

```sh
cargo build --example http-routing-a-wasm --target wasm32-wasip2
cargo build --example http-routing-b-wasm --target wasm32-wasip2
```

## Run the host

```sh
export RUST_LOG="info,opentelemetry_sdk=off,omnia_wasi_http=debug"
cargo run --example http-routing -- run --config examples/http-routing/omnia.toml
```

The server listens on `localhost:8080`.

## Try it

```sh
curl localhost:8080/a    # -> http-routing example: guest a
curl localhost:8080/b    # -> http-routing example: guest b
curl -i localhost:8080/c # -> HTTP/1.1 404 Not Found (no route matched)
```
