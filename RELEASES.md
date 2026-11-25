## 0.39.0

Unreleased

### Added

- Per-guest outbound host grants via `Grants::hosts` in `[guest.grants]`, `[plugins.grants]`, and `runtime!` `grants: { hosts }`, with `Grants::{allows_host, covers_host}` and `omnia_core::host_pattern` for capability crates. A list holds outbound `wasi:http` requests and the redirects `HttpDefault` follows (`http-request-denied`), `omnia:model` worker `egress` (`denied`), and the `registry` an `omnia:plugins` `release` load names (`denied`). It matches plain host names alone, so `*` never reaches an address ([#423](https://github.com/augentic/omnia/issues/423)).
- Per-guest guest grants via `Grants::guests` in `[guest.grants]`, `[plugins.grants]`, and `runtime!` `grants: { guests }`, with `Grants::allows_guest`. A list holds each link call a guest makes, which fails before its target loads, and each `omnia:plugins` load, which is `denied` by the name its source registers under before anything is read or fetched ([#422](https://github.com/augentic/omnia/issues/422)).

### Changed

- **Host views take the guest's grants:** `HostCtx::view` and `omnia_wasi_http::HttpBorrow::as_view` take `&Arc<Grants>`, so a hand-written host or HTTP backend adds the parameter, and `#[derive(HostCtx)]` leaves it unused ([#423](https://github.com/augentic/omnia/issues/423)).
- **Grants tables without `hosts` or `guests`:** a 0.38 `[guest.grants]` or `[plugins.grants]` table that leaves out `hosts` reaches no host, and one that leaves out `guests` reaches no guest. Their guests' outbound HTTP and worker egress, or link calls and loads, fail when made, not at boot. Add `hosts` to every table whose guests call out, as Emery's `[plugins.grants]` must. Add `guests` to every table whose guests call or load another guest, or `guests = ["*"]` to keep reaching every guest ([#423](https://github.com/augentic/omnia/issues/423), [#422](https://github.com/augentic/omnia/issues/422)).
- **Worker egress and test doubles:** an `omnia:model` worker `egress` entry ending in a numeric label (`2130706433`, `example.1`) is `invalid`, since it reads as an IP address, and `omnia_test::host::MatchedHttp` refuses a request outside the guest's `hosts` with `http-request-denied`, recording none ([#423](https://github.com/augentic/omnia/issues/423)).
- **Hand-bound handlers import `omnia:otel`:** `omnia_sdk::exports::messaging::export!` and `omnia_sdk::exports::websocket::export!` run each delivery or event inside the guest telemetry scope, so a guest that binds either imports `omnia:otel` and its deployment links `WasiOtel`. The no-op `OtelDefault` suffices.
- **Only the command run receives the command's arguments:** a guest the run calls or loads, a guest a host dispatch reaches, and a guest a trigger serves see an empty argv, whatever their grants. A `mode: server` deployment given arguments fails at startup. A guest that read the arguments outside the run takes its input in the call, or from `[env] inherit` or `omnia:config` ([#437](https://github.com/augentic/omnia/issues/437)).
- **`ModelClient::limits` has no default:** every backend states the session limits the host holds it to. A backend that took the default returns `Limits::default()`, and a client that wraps another returns the wrapped client's ([#435](https://github.com/augentic/omnia/issues/435)).
- **`ScriptedModel::limited` sets a script's limits:** `omnia_test::host::ScriptedModel::limits(self, Limits)` is renamed, so a double that wraps a `ScriptedModel` forwards its limits as `self.inner.limits()` ([#435](https://github.com/augentic/omnia/issues/435)).
- **`Runtime::enter` takes the invocation's slot:** `Runtime::enter(guest, chain, slot)` takes the `Slot` a trigger got from `Runtime::slot` or `Runtime::try_slot`, and returns an `Entered` guard in place of the `Store`. The guard implements `AsContextMut`, so `&mut entered` passes wherever `&mut store` did, and `store.run_concurrent(..)` becomes `entered.as_context_mut().run_concurrent(..)`. Dropping the guard drops the store, then frees the slot, so a trigger an embedder runs itself counts against `MAX_CONCURRENT_INVOCATIONS`. A slot taken from another runtime's gate panics ([#446](https://github.com/augentic/omnia/issues/446)).

### Fixed

- Record a `tracing` event's `monotonic_counter.`, `counter.`, `histogram.`, or `gauge.` metric whatever `RUST_LOG` is, in a guest and on the host. Below the run's level such an event was dropped, so a guest's `info!` counter never reached a server running at its default `warn`. `RUST_LOG` still governs the console and spans, and under `tracing-log` a `log` record still dispatches only at the run's level ([#426](https://github.com/augentic/omnia/issues/426)).
- Compile `#[omnia_sdk::otel::instrument]` on an `async fn`, `otel::scope`, `otel::flush`, `command::execute`, `http::serve`, and `messaging::handle` on a native target. Each was `wasm32`-only, so a guest crate that instrumented a handler did not build natively. On a native target `scope` installs no telemetry, so an instrumented handler's span reaches the test's own subscriber.

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
