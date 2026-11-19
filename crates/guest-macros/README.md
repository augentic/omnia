# omnia-guest-macros

Procedural macros for Omnia guests:

- **`#[instrument]`** — wraps a function in an OpenTelemetry span. On an async function it also runs the body inside `omnia_sdk::otel::scope`, so the outermost instrumented async function initializes the guest's telemetry and exports it as it returns; a sync function only opens the span. Guests reach it as `omnia_sdk::otel::instrument`.
- **`#[derive(Settings)]`** — derives a `KEYS` list and an `async fn load()` that reads each field through `omnia_sdk::config::require`. Re-exported as `omnia_sdk::Settings`.
- **`#[derive(FromRow)]`** — derives `omnia_sdk::sql::FromRow`, reading each field from the column of the same name. Re-exported as `omnia_sdk::sql::FromRow`.

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
