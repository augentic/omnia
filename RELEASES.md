## 0.39.0

Unreleased

### Added

- Per-guest outbound host grants via `Grants::hosts` in `[guest.grants]`, `[plugins.grants]`, and `runtime!` `grants: { hosts }`, with `Grants::{allows_host, covers_host}` and `omnia_core::host_pattern` for capability crates. A list holds outbound `wasi:http` requests and the redirects `HttpDefault` follows (`http-request-denied`), `omnia:model` worker `egress` (`denied`), and the `registry` an `omnia:plugins` `release` load names (`denied`). It matches plain host names alone, so `*` never reaches an address ([#423](https://github.com/augentic/omnia/issues/423)).
- Per-guest guest grants via `Grants::guests` in `[guest.grants]`, `[plugins.grants]`, and `runtime!` `grants: { guests }`, with `Grants::allows_guest`. A list holds each link call a guest makes, which fails before its target loads, and each `omnia:plugins` load, which is `denied` by the name its source registers under before anything is read or fetched ([#422](https://github.com/augentic/omnia/issues/422)).
- `MESSAGING_MAX_IN_FLIGHT` and `RuntimeOptions::messaging_max_in_flight`, the most message deliveries that hold or wait for a slot of the concurrency gate at once. Unset, it is half the gate, rounded up. `RuntimeOptions::concurrent_invocations` and `RuntimeOptions::messaging_in_flight` give the gate's width and messaging's share ([#428](https://github.com/augentic/omnia/issues/428)).

### Changed

- **Host views take the guest's grants:** `HostCtx::view` and `omnia_wasi_http::HttpBorrow::as_view` take `&Arc<Grants>`, so a hand-written host or HTTP backend adds the parameter, and `#[derive(HostCtx)]` leaves it unused ([#423](https://github.com/augentic/omnia/issues/423)).
- **Grants tables without `hosts` or `guests`:** a 0.38 `[guest.grants]` or `[plugins.grants]` table that leaves out `hosts` reaches no host, and one that leaves out `guests` reaches no guest. Their guests' outbound HTTP and worker egress, or link calls and loads, fail when made, not at boot. Add `hosts` to every table whose guests call out, as Emery's `[plugins.grants]` must. Add `guests` to every table whose guests call or load another guest, or `guests = ["*"]` to keep reaching every guest ([#423](https://github.com/augentic/omnia/issues/423), [#422](https://github.com/augentic/omnia/issues/422)).
- **Worker egress and test doubles:** an `omnia:model` worker `egress` entry ending in a numeric label (`2130706433`, `example.1`) is `invalid`, since it reads as an IP address, and `omnia_test::host::MatchedHttp` refuses a request outside the guest's `hosts` with `http-request-denied`, recording none ([#423](https://github.com/augentic/omnia/issues/423)).
- **Hand-bound handlers import `omnia:otel`:** `omnia_sdk::exports::messaging::export!` and `omnia_sdk::exports::websocket::export!` run each delivery or event inside the guest telemetry scope, so a guest that binds either imports `omnia:otel` and its deployment links `WasiOtel`. The no-op `OtelDefault` suffices.
- **Only the command run receives the command's arguments:** a guest the run calls or loads, a guest a host dispatch reaches, and a guest a trigger serves see an empty argv, whatever their grants. A `mode: server` deployment given arguments fails at startup. A guest that read the arguments outside the run takes its input in the call, or from `[env] inherit` or `omnia:config` ([#437](https://github.com/augentic/omnia/issues/437)).
- **`ModelClient::limits` has no default:** every backend states the session limits the host holds it to. A backend that took the default returns `Limits::default()`, and a client that wraps another returns the wrapped client's ([#435](https://github.com/augentic/omnia/issues/435)).
- **`ScriptedModel::limited` sets a script's limits:** `omnia_test::host::ScriptedModel::limits(self, Limits)` is renamed, so a double that wraps a `ScriptedModel` forwards its limits as `self.inner.limits()` ([#435](https://github.com/augentic/omnia/issues/435)).
- **`Runtime::enter` takes the invocation's slot:** `Runtime::enter(guest, chain, slot)` takes the `Slot` a trigger got from `Runtime::slot` or `Runtime::try_slot`, and returns an `Entered` guard in place of the `Store`. The guard implements `AsContextMut`, so `&mut entered` passes wherever `&mut store` did, and `store.run_concurrent(..)` becomes `entered.as_context_mut().run_concurrent(..)`. Dropping the guard drops the store, then frees the slot, so a trigger an embedder runs itself counts against `MAX_CONCURRENT_INVOCATIONS`. A slot taken from another runtime's gate panics ([#446](https://github.com/augentic/omnia/issues/446)).
- **`Handler::max_in_flight` is required:** an `omnia_wasi_messaging::Handler` states the most messages it takes at once, and `dispatch` reads no further from its stream while that many are with the handler. A hand-written handler returns its bound. A backend whose `Endpoint::consume` runs its own delivery loop keeps no more than `max_in_flight` messages with the handler ([#428](https://github.com/augentic/omnia/issues/428)).
- **Messaging takes half the gate by default:** message deliveries hold or wait for at most half the slots of the concurrency gate, rounded up, so a deployment that serves only messaging runs half as many at once. Set `MESSAGING_MAX_IN_FLIGHT` to the gate's width to run as many as 0.38 did. Messages past the share wait in the transport. NATS core drops them once async-nats's buffer fills, and the in-memory broker once it falls 32 behind ([#428](https://github.com/augentic/omnia/issues/428)).
- **Every importer of a linked function agrees:** guests that import one linked function with different signatures, a parameter's name included, no longer stand together. A deployment that loads two such guests at boot fails, and a later one is refused. 0.38 compared importers' asyncness alone. Build every importer of a function from the same WIT ([#431](https://github.com/augentic/omnia/issues/431)).
- **Public API nothing used is withdrawn:** `Deployment::assemble` is the one way to build a runtime, and only the runtime builds a store context ([#448](https://github.com/augentic/omnia/issues/448)).
  - Removed: `Runtime::registry_config` and `RuntimeParts::registry_config`, `Runtime::http_trigger_router`, `Deployment::selector`, `GuestSelector`, `FirstArgSelector`, `CliRoutes`, `Routes::cli`, and the `ModelClient` and `VcsClient` impls for a boxed client. `Routes::new` takes the HTTP, messaging and WebSocket tables, and `InProcessLinks::new` takes the `ChainPolicy` alone.
  - Crate-private: `StoreConfig`, `StoreBase::new`, `StoreBase`'s fields, `Guest::local` and `Deployment::into_registry`.
  - Hidden from the docs, for omnia's own crates: `RuntimeParts`, `Runtime::from_parts`, `RegistryParts`, `Registry::assemble`, `Runtime::serve_links`, `StoreFactory`, `LinkSeam`, `NoLinks` and `Plugins::install`.
  - No longer re-exported from `omnia`: `RuntimeParts`, `RegistryParts`, `StoreConfig`, `StoreFactory`, `LinkSeam`, `NoLinks`, `CliRoutes`, `InProcessLinks`, `GuestSelector`, `FirstArgSelector`, `Cli`, `Command` and `Parser`.
  - `omnia_cli::plan` returns a `Plan`, either `Plan::Run` or `Plan::Compile`.
- **Two unmarked command guests fail:** when two or more boot guests export `wasi:cli/run` and none is marked `command = true`, command mode fails naming them and advising the mark. It failed with "trigger `cli` has 2 capable guests … but no routes", which sent the operator to routes the cli trigger cannot take ([#448](https://github.com/augentic/omnia/issues/448)).
- **The generated `main` runs `compile`:** `<runtime> compile <wasm> [-o <path>] [-t <triple>]` compiles under the compile-affecting settings its environment sets, so a `run` under the same environment loads the artifact. A runtime built without omnia's `jit` feature refuses it, naming the feature. It was refused as "the generated `main` only supports `run`" ([#448](https://github.com/augentic/omnia/issues/448)).
- **A call through `Runtime::dispatcher()` takes a slot:** each call is a root invocation, so it waits at a full concurrency gate and counts against `MAX_CONCURRENT_INVOCATIONS`, freeing its slot once the callee's store drops. A store's own dispatcher takes none. A host-side caller that holds a slot while it dispatches waits on itself at a full gate, so it drops the slot first ([#448](https://github.com/augentic/omnia/issues/448)).

### Fixed

- Record a `tracing` event's `monotonic_counter.`, `counter.`, `histogram.`, or `gauge.` metric whatever `RUST_LOG` is, in a guest and on the host. Below the run's level such an event was dropped, so a guest's `info!` counter never reached a server running at its default `warn`. `RUST_LOG` still governs the console and spans, and under `tracing-log` a `log` record still dispatches only at the run's level ([#426](https://github.com/augentic/omnia/issues/426)).
- Compile `#[omnia_sdk::otel::instrument]` on an `async fn`, `otel::scope`, `otel::flush`, `command::execute`, `http::serve`, and `messaging::handle` on a native target. Each was `wasm32`-only, so a guest crate that instrumented a handler did not build natively. On a native target `scope` installs no telemetry, so an instrumented handler's span reaches the test's own subscriber.
- Stop an HTTP guest whose request is dropped before its response head. The guest ran on with no wall-clock bound, holding its slot of the concurrency gate until it answered, or for good if it never did. An HTTP guest's `GUEST_TIMEOUT_MS` now counts from when it takes its slot, so it bounds instantiation too ([#463](https://github.com/augentic/omnia/issues/463)).
- Stamp an HTTP request's `x-deadline` when it takes its slot of the concurrency gate, the instant its `GUEST_TIMEOUT_MS` counts from. It was stamped before the request was routed, so a declared guest's first request was told a deadline its own load had already spent from ([#427](https://github.com/augentic/omnia/issues/427)).
- Keep HTTP and WebSocket served under a backlog of message deliveries. Deliveries waiting for a slot of the concurrency gate take every freed slot first, and `dispatch` handed the host every message it read, so a burst refused requests and events until it drained, and sustained load refused them for good. Messaging now holds or waits for no more than its share of the gate ([#428](https://github.com/augentic/omnia/issues/428)).
- Load a declared guest once when its first uses arrive together. Each acquired and compiled the guest itself, and HTTP and WebSocket resolve a guest before they take a slot of the concurrency gate, so a burst of cold requests ran a fetch and a compile per request that the gate bounded none of. The first uses now share one load and its outcome, error included, and dropping one fails none of the others. A panic in the load reaches each of them as `GuestError::Internal` ([#432](https://github.com/augentic/omnia/issues/432)).
- Settle racing admissions of one guest identity as the one that seated first. With omnia's `link` feature, a `Runtime::register`, `Runtime::admit_bytes`, or `Runtime::admit` that raced another of the same identity could fail as `GuestError::Internal`, and an `omnia:plugins` load of a path or package as `LoadError::Other`. A racing registration now attests the same bytes and grants or reports `GuestError::Conflict`, and a racing load attests the same bytes or answers a `conflict` ([#432](https://github.com/augentic/omnia/issues/432)).
- Refuse a guest that disagrees with a registered guest on a linked function's signature, whichever of the two seats first. Only each function's first boot importer was checked against its exporter, so a late importer, a second importer, and every pair under a manifest of path guests went unchecked until a call crossed the skew. Such a call could trap after the exporter had run, or read a result the exporter never returned as `false`. A refused seat is `GuestError::Invalid`, not `GuestError::Internal`, and an `omnia:plugins` load answers `invalid`, not `other` ([#431](https://github.com/augentic/omnia/issues/431)).

---

Release notes for previous releases can be found on the respective release branches of the repository.

<!-- ARCHIVE_START -->
* [0.38.x](https://github.com/augentic/omnia/blob/release-0.38.0/RELEASES.md)
* [0.37.x](https://github.com/augentic/omnia/blob/release-0.37.0/RELEASES.md)
* [0.36.x](https://github.com/augentic/omnia/blob/release-0.36.0/RELEASES.md)
* [0.35.x](https://github.com/augentic/omnia/blob/release-0.35.0/RELEASES.md)
* [0.34.x](https://github.com/augentic/omnia/blob/release-0.34.0/RELEASES.md)
* [0.33.x](https://github.com/augentic/omnia/blob/release-0.33.0/RELEASES.md)
* [0.32.x](https://github.com/augentic/omnia/blob/release-0.32.0/RELEASES.md)
* [0.31.x](https://github.com/augentic/omnia/blob/release-0.31.0/RELEASES.md)
* [0.30.x](https://github.com/augentic/omnia/blob/release-0.30.0/RELEASES.md)
* [0.29.x](https://github.com/augentic/omnia/blob/release-0.29.0/RELEASES.md)
* [0.28.x](https://github.com/augentic/omnia/blob/release-0.28.0/RELEASES.md)
* [0.27.x](https://github.com/augentic/omnia/blob/release-0.27.0/RELEASES.md)
* [0.25.x](https://github.com/augentic/omnia/blob/release-0.25.0/RELEASES.md)
* [0.23.x](https://github.com/augentic/omnia/blob/release-0.23.0/RELEASES.md)
* [0.22.x](https://github.com/augentic/omnia/blob/release-0.22.0/RELEASES.md)
* [0.21.x](https://github.com/augentic/omnia/blob/release-0.21.0/RELEASES.md)
* [0.20.x](https://github.com/augentic/omnia/blob/release-0.20.0/RELEASES.md)
* [0.19.x](https://github.com/augentic/omnia/blob/release-0.19.0/RELEASES.md)
* [0.18.x](https://github.com/augentic/omnia/blob/release-0.18.0/RELEASES.md)
* [0.17.x](https://github.com/augentic/omnia/blob/release-0.17.0/RELEASES.md)
* [0.16.x](https://github.com/augentic/omnia/blob/release-0.16.0/RELEASES.md)
* [0.15.x](https://github.com/augentic/omnia/blob/release-0.15.0/RELEASES.md)
