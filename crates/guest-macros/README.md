# omnia-guest-macros

Procedural macros for Omnia guests:

- **`#[instrument]`** — wraps a function in an OpenTelemetry span. On an async function it also runs the body inside `omnia_wasi_otel::scope`, so the outermost instrumented async function initializes the guest's telemetry and exports it as it returns; a sync function only opens the span. Guests reach it as `omnia_wasi_otel::instrument`.
- **`#[derive(Provider)]`** — derives capability traits on a struct by delegating each capability to the field annotated with `#[provider(..)]`. A bare name is an `omnia_sdk` capability (`#[provider(Config, StateStore)]`); a path names any other `#[capability]` trait (`#[provider(crate::Geocoder)]`). Re-exported as `omnia_sdk::Provider`. The field's type is part of the provider's interface, so a `pub` provider delegates only to `pub` field types.
- **`#[capability]`** — makes a trait a capability `#[derive(Provider)]` can delegate. It emits the hidden `Has<Trait>` accessor trait, a blanket forwarding impl, and pointer impls (`Arc<P>`, `&P`, `Box<P>`) beside the trait. The accessor trait names the type it delegates to as `Inner`, and each associated type of the capability (such as `StateStore::Named`) forwards to `Inner`'s. Re-exported as `omnia_sdk::capability`. A crate that re-exports its trait re-exports `Has<Trait>` beside it, so a provider names the trait by that path.
- **`#[derive(Settings)]`** — derives a `KEYS` list and an `async fn load(config)` that reads each field through `omnia_sdk::ConfigExt::require`. Re-exported as `omnia_sdk::Settings`.
- **`#[derive(FromRow)]`** — derives `omnia_sdk::sql::FromRow`, reading each field from the column of the same name. Re-exported as `omnia_sdk::sql::FromRow`.

A guest-side library defines a capability of its own and a provider delegates to it by path:

```rust,ignore
use omnia_sdk::{Provider, capability};

#[capability]
pub trait Geocoder: Send + Sync {
    fn locate(&self, place: &str) -> impl Future<Output = Option<(f64, f64)>> + Send;
}

#[derive(Provider)]
struct Services {
    #[provider(Config)]
    config: omnia_test::guest::MapConfig,
    #[provider(crate::Geocoder)]
    geocoder: FixedGeocoder,
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
