## 0.37.0

Released 2026-10-21

### Added

- `omnia-compile`: ahead-of-time guest compilation as a leaf crate (`CompileOptions`, `compile`, default `cranelift` feature); `pooling-allocator` scoped to `omnia-core` and `omnia` ([#350](https://github.com/augentic/omnia/pull/350)).
- Process-wide tracing: verbosity flags compose with `RUST_LOG`, propagate to every guest's WASI environment, and drive host console levels for command vs server mode ([#317](https://github.com/augentic/omnia/pull/317), [#307](https://github.com/augentic/omnia/pull/307), [#319](https://github.com/augentic/omnia/pull/319)).
- `ChainCtx` on the guest store for dispatch-chain scope; link relay reads caller context via `HasChain` ([#310](https://github.com/augentic/omnia/pull/310)).
- `omnia-wasi-vcs` (`omnia:vcs`): store, workspace, and transport over lent mount locations ([#345](https://github.com/augentic/omnia/pull/345)).
- Model `worker-grant` on `omnia:model@0.2.0` (egress, read paths, env for completion workers) ([#352](https://github.com/augentic/omnia/pull/352)).
- `omnia_sdk::api::messaging::Routes` and `Router::merge(routes)` for exact-topic messaging handlers ([#347](https://github.com/augentic/omnia/pull/347), [#376](https://github.com/augentic/omnia/pull/376)).
- `POOL_IDLE_RELEASE_MS` / `RuntimeOptions::pool_idle_release` to return warm pool memory after idle ([#343](https://github.com/augentic/omnia/pull/343)).
- `StoreConfig::stdio` / `DeploymentBuilder::stdio` and test-host captured stdout/stderr ([#344](https://github.com/augentic/omnia/pull/344)).
- `runtime!` embeds guests (`guests:`, `plugins:`, `registries:`), sets program name from `CARGO_PKG_NAME`, and forwards env for command guests ([#310](https://github.com/augentic/omnia/pull/310), [#309](https://github.com/augentic/omnia/pull/309), [#306](https://github.com/augentic/omnia/pull/306)).

### Changed

- **Guest loading and plugins:** unified wasm/precompiled loader, lazy first-use guests, digest pins, `[plugins]` store + registry routing, loader `declared` locations, read-only mount policy for `path` loads, export lists on loaded plugins ([#312](https://github.com/augentic/omnia/pull/312), [#315](https://github.com/augentic/omnia/pull/315), [#316](https://github.com/augentic/omnia/pull/316), [#344](https://github.com/augentic/omnia/pull/344)).
- **Manifest and CLI:** `[[guest]] name`, `manifest:` / `-m` / `OMNIA_MANIFEST`, `loader` feature (was `plugin`); dropped `services`, `link:`/`plugin:` blocks, and `--link` ([#310](https://github.com/augentic/omnia/pull/310), [#348](https://github.com/augentic/omnia/pull/348)).
- **Linking:** host vs guest interfaces by namespace (`wasi:`/`omnia:` host); guest package namespaces no longer collide ([#310](https://github.com/augentic/omnia/pull/310)).
- **Telemetry:** OTLP always compiled (`otlp` feature removed); collectors only when endpoints are set; no default localhost exporter; console colour only on TTY; `cli-run` and trigger spans at `info` ([#314](https://github.com/augentic/omnia/pull/314), [#313](https://github.com/augentic/omnia/pull/313), [#320](https://github.com/augentic/omnia/pull/320)).
- **HTTP:** richer default client error codes; shared client for per-request timeouts ([#343](https://github.com/augentic/omnia/pull/343)).
- **Model:** improved answer extraction (`Format::candidate`); `Usage` is the WIT record ([#329](https://github.com/augentic/omnia/pull/329), [#346](https://github.com/augentic/omnia/pull/346)).
- Upgraded **wasmtime** / `wasmtime-wasi*` to **50.0.0** (rebuild precompiled artifacts; fuel for composite values; stricter WASI HTTP/stream behaviour) ([#385](https://github.com/augentic/omnia/pull/385)).
- Shared **augentic/toolkit** CI, mise tasks, and repo conventions ([#321](https://github.com/augentic/omnia/pull/321), [#322](https://github.com/augentic/omnia/pull/322), [#323](https://github.com/augentic/omnia/pull/323), [#338](https://github.com/augentic/omnia/pull/338), [#340](https://github.com/augentic/omnia/pull/340), [#341](https://github.com/augentic/omnia/pull/341), [#351](https://github.com/augentic/omnia/pull/351)).
- `cfg_select!` replaces stacked `#[cfg]` attributes ([#372](https://github.com/augentic/omnia/pull/372)).
- Dependency updates and test-suite performance ([#373](https://github.com/augentic/omnia/pull/373), [#349](https://github.com/augentic/omnia/pull/349)).
- Documentation and lint cleanups ([#318](https://github.com/augentic/omnia/pull/318), [#339](https://github.com/augentic/omnia/pull/339), [#342](https://github.com/augentic/omnia/pull/342)).

### Removed

- `OTEL_GRPC_URL` (use OpenTelemetry `OTEL_EXPORTER_OTLP_*` variables) ([#314](https://github.com/augentic/omnia/pull/314)).
- `omnia_wasi_otel::set_filter` (process-composed `RUST_LOG` replaces per-guest filter API) ([#317](https://github.com/augentic/omnia/pull/317)).
- `RegistryClient::cached`, `ContentStore`, and `ReleaseStore` in favour of `PackageStore` ([#344](https://github.com/augentic/omnia/pull/344)).
- Per-start re-fetch of package guests when the store already holds the release ([#344](https://github.com/augentic/omnia/pull/344)).
- Renovate config; Dependabot ignores pinned toolkit refs ([#374](https://github.com/augentic/omnia/pull/374), [#375](https://github.com/augentic/omnia/pull/375)).

**Full Changelog**: https://github.com/augentic/omnia/compare/v0.36.0...v0.37.0

---

Release notes for previous releases can be found on the respective release branches of the repository.

<!-- ARCHIVE_START -->
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
