# Multi-guest HTTP routing

Two HTTP guests (`a` and `b`) behind path prefixes, wired through a deployment manifest. The host runs a single HTTP server; each guest's `routes.http` prefixes select it per request by longest-prefix match.

Because two guests export `wasi:http/handler`, the routes are **required** — without them startup fails with an ambiguity error. With a single guest (as in the other examples) routing falls back to a catch-all and no manifest is needed.

## Quick Start

This example deploys two guests from a manifest, so build and run stay manual:

```bash
# build the guests
cargo build --example http-routing-a-wasm --target wasm32-wasip3
cargo build --example http-routing-b-wasm --target wasm32-wasip3

# run the host — the manifest path is compiled in (runtime! `manifest:`),
# so a bare `run` works from any directory
cargo run --example http-routing -- -v run

# or with an explicit manifest
cargo run --example http-routing -- -v run --manifest examples/http-routing/omnia.toml
```

The server listens on `localhost:8080`.

## Try it

```sh
curl localhost:8080/a    # -> http-routing example: guest a (HTTP_ADDR=unset)
curl localhost:8080/b    # -> http-routing example: guest b
curl -i localhost:8080/c # -> HTTP/1.1 404 Not Found (no route matched)
```

Guest `a` reports `HTTP_ADDR` because the manifest's `[env] inherit` names
it. Run with `HTTP_ADDR=127.0.0.1:8080` and it reports that address. A
variable the manifest does not name never reaches a guest.
