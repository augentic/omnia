# omnia-guest-macros

Procedural attributes for Omnia guests: `#[handler]` derives an `omnia_guest::api::Handler` implementation from a bare handler function, and the independent `#[instrument]` attribute wraps a function in an OpenTelemetry span and initializes the guest subscriber on entry. Routing and WASI exports are ordinary Rust APIs in `omnia-guest`.

## Handlers

`#[handler]` (re-exported as `omnia_guest::handler`) turns an `async fn` into a handler; it takes no arguments. The first parameter is the owned input type and becomes the impl target (`Self` is the input); the second must be `Context<'_, P>`; the return type is `Result<T>` (`omnia_guest::Result`, error defaults to `omnia_guest::Error`) or `Result<T, E>`. The fn's generics and bounds are reused verbatim, and the fn itself is re-emitted unchanged (attributes included) so other handlers can call it directly.

The generated `handle` is a bare delegation with no instrumentation. When a span is wanted, stack `#[tracing::instrument]` on the fn itself:

```rust,ignore
#[omnia_guest::handler]
#[tracing::instrument(skip_all)]
async fn motion_message<P>(input: MotionMessage, context: Context<'_, P>) -> Result<()>
where
    P: Send + Sync + 'static,
{
    // ...
}
```

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
