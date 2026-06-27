# Multi-guest HTTP routing

Two HTTP guests (`a` and `b`) behind path prefixes, wired through a deployment
manifest. The host runs a single HTTP server; the manifest's `[[route.http]]`
table selects the guest per request by longest-prefix match.

Because two guests export `wasi:http/incoming-handler`, the routes are
**required** — without them startup fails with an ambiguity error. With a single
guest (as in the other examples) routing falls back to a catch-all and no
manifest is needed.

## Build the guests

```sh
cargo build --example routing-a-wasm --target wasm32-wasip2
cargo build --example routing-b-wasm --target wasm32-wasip2
```

## Run the host

```sh
cargo run --example routing -- run --config examples/routing/omnia.toml
```

The server listens on `localhost:8080`.

## Try it

```sh
curl localhost:8080/a    # -> routing example: guest a
curl localhost:8080/b    # -> routing example: guest b
curl -i localhost:8080/c # -> HTTP/1.1 404 Not Found (no route matched)
```
