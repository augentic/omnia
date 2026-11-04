# omnia-guest-macros

Procedural macros for Omnia guests:

- **`#[instrument]`** — wraps a function in an OpenTelemetry span. On an async function it also runs the body inside `omnia_wasi_otel::scope`, so the outermost instrumented async function initializes the guest's telemetry and exports it as it returns; a sync function only opens the span. Guests reach it as `omnia_wasi_otel::instrument`.
- **`#[derive(Provider)]`** — derives `omnia_sdk` capability traits on a struct by delegating each named capability to the field annotated with `#[provider(Capability, ...)]`. Re-exported as `omnia_sdk::Provider`.
- **`#[capability]`** — internal attribute on `omnia_sdk` capability traits emitting the hidden `Has<Capability>` accessor trait, blanket forwarding impl, and pointer impls (`Arc<P>`, `&P`, `Box<P>`).

## Instrumentation

```rust,ignore
use omnia_guest_macros::instrument;

#[instrument]
fn handle() {
    // a span named "handle" is active for the duration of this body
}

#[instrument(name = "custom_span", level = Level::DEBUG)]
async fn process() {
    // async bodies are instrumented too
}
```

Accepted arguments:

- `name` -- overrides the span name (defaults to the function name)
- `level` -- sets the span level (e.g. `Level::DEBUG`; defaults to `INFO`)
