# Design: Dynamic Guest Registration

> Status: Landed — lets an embedder grow (and shrink) the guest registry after startup through an explicit async registration primitive: verify → compile → pre-instantiate → serve → publish, at install time. Registration is the primitive; the lazy resolve-on-miss hook (`GuestResolver`) and the dynamic HTTP trigger fallback (§4.5) are a thin layer over it. Complements — does not replace — the static `[[guest]]` population; a deployment that never registers behaves exactly as today. Depends: the guest registry (`crates/omnia/src/registry.rs`), host-mediated dispatch (`crates/omnia/src/dispatch/`), the programmatic `Manifest` / `DeploymentBuilder` (`crates/omnia/src/deployment/`), `omnia compile` (`crates/omnia/src/options/compile.rs`). Relates: [embedded-guest](embedded-guest.md) (the opposite trade: fully static composition at build time; this design grows composition at run time).

## 1. Motivation

The registry is frozen at assembly: `Registry::assemble` pre-instantiates every loaded guest into a `BTreeMap`, validates the route tables against it, and the only lookup thereafter is `Registry::get` → `Option`. A deployment that *acquires* a component after startup — an OCI pull, a package-manager install, a plugin dropped into a watched directory — cannot dispatch to it without a restart.

Growing a running host's component set is the standard shape for dynamic component platforms:

- **wasmCloud** references components by OCI identity and fetches, verifies, and instantiates them on demand across the lattice; the running host's component set is not fixed at boot. <https://wasmcloud.com/docs/concepts/components/>
- **Envoy** loads Wasm extensions from a remote data source at runtime and **requires a SHA-256 checksum** before the fetched module may be compiled — the same verify-before-load, fail-closed posture required here. <https://www.envoyproxy.io/docs/envoy/latest/api-v3/extensions/wasm/v3/wasm.proto>
- **Kubernetes** schedules workloads whose images are pulled on first use and pinned by digest; the node's runnable set grows after kubelet start.
- **Fermyon Spin** resolves application components from OCI registries at run, keyed by exact reference. <https://spinframework.dev/v3/registry-tutorial>
- Plugin hosts generally (VS Code activation events, Envoy filter chains) admit extension code while running rather than enumerating it at process start.

Omnia already treats guest identity as *data*: `GuestId` is an opaque ordered key the core never parses (`registry.rs` module doc), the dispatch selector reads the target id per call, and the manifest doc anticipates OCI as "another source kind". A registry that can grow while running is the natural completion of that stance; the frozen map is the anomaly.

The key observation shaping this design: in the motivating scenario — an embedder installs a component mid-run and calls it in the same process — **the embedder knows the moment the component exists**. It does not need the runtime to lazily discover the component inside a dispatch; it needs a way to hand the component to the runtime. A *push* (registration) primitive serves that directly and keeps the dispatch path untouched. The *pull* (resolve-on-miss) shape earns its extra machinery only when no host-side code observes the install: external traffic naming a component nothing local has acted on yet (dynamic HTTP fan-out), or an installer that is itself a *guest* — writing the component through a store mount mid-dispatch, with no host API to call and the embedder blocked inside `run`. Both are served by §4.5.

## 2. Current state: what exists, what is missing

What exists:

- **Identity-keyed dispatch.** Every path already resolves a `GuestId` per call: the dispatch selector for host-mediated links, the per-trigger `Router` for HTTP/messaging/websocket, the sole-exporter rule for CLI.
- **Programmatic deployment.** `Manifest` / `DeploymentBuilder` assemble the whole deployment in memory.
- **Loading machinery.** `Source::load` compiles or deserializes bytes; `Linker::instantiate_pre` against the shared linker is a per-guest operation; `omnia compile` already produces settings-matched pre-compiled artifacts.

What is missing — three structures freeze at bootstrap:

1. **The registry map.** `Registry::assemble` builds the `BTreeMap` once and drops the `Linker`; there is no insert or remove path and no retained linker to pre-instantiate against (`registry.rs`). `get` hands out `&Guest<T>` borrowed from the frozen map, which no concurrent-insert structure can do.
2. **The link serve side.** `serve_links` walks the registered guests once at startup, builds one `InProcServer` per exporting guest, and installs the `InProcess` carrier into a `OnceLock` (`dispatch/serve.rs`, `dispatch/handle.rs::install`). A guest inserted later that exports a linked interface has no server: `InProcess::connect` misses its map and the dispatch fails even though the guest is registered.
3. **Trigger routing.** `Router::build` validates every static route target against the registered set at bootstrap (`registry/routing.rs`); there is no deployment-supplied fallback from a request key to an identity. (Landed with resolve-on-miss, §4.5.)

## 3. Constraints

- **Law 2 holds.** The runtime core sees opaque guest ids and bytes. Where components live, how ids map to filenames or references, and what verification means are *deployment policy*: the embedder verifies (digest, signature, provenance) before calling `register`. The core never gains a store layout, a filename convention, or consumer vocabulary.
- **Fail closed.** The runtime registers exactly what it is handed and nothing else: no directory scan, no network probe, no fallback acquisition. An unregistered id is a dispatch error on the caller, same as today.
- **Static entries win.** `register` refuses an id that already names a static `[[guest]]` entry; `deregister` refuses a static entry. Only dynamically registered guests can be replaced or removed.
- **Registration cannot widen the allow-list.** The shared linker's host set and the deployment's `link` union are fixed at bootstrap. A registered guest may only import host interfaces already linked and only participate in link interfaces already in the `link` union. The allow-list stays a deployment declaration, not something a registered component can widen.
- **Serve before publish.** A registration is observable (present in the map, single-flight waiters released in §4.5) only after its link serve side is wired, so no caller can resolve the entry and then miss the wRPC endpoint. As implemented, entry and endpoint are published as one atomic lifecycle transition (a shared lifecycle gate over the registry map and the transport's endpoint map), which also makes concurrent register/deregister linearizable.
- **Instance-per-call is unchanged.** Registered guests are pre-instantiated once and instantiated fresh per call like every static guest — which is also what makes replace/remove safe: an in-flight call holds its own clone of the old `InstancePre` and completes on it.

## 4. Proposed design

### 4.1 The registration primitive (`Runtime::register` / `Runtime::deregister`)

```rust
impl<B> Runtime<B> {
    /// Admit a verified component under `id`: compile (or deserialize),
    /// pre-instantiate against the shared host set, wire its link serve side,
    /// then publish it in the registry. Fails without side effects if `id` is
    /// already registered or the component's imports exceed the deployment's
    /// linked host set and `link` union.
    pub async fn register(&self, id: GuestId, artifact: GuestArtifact) -> Result<()>;

    /// Remove a dynamically registered guest. In-flight calls complete on the
    /// instance they hold; new dispatches to `id` fail as unregistered.
    /// Refused for static `[[guest]]` entries.
    pub async fn deregister(&self, id: &GuestId) -> Result<()>;
}

/// Component bytes the embedder has already verified (Law 2: verification is
/// deployment policy and happens before the runtime sees the bytes). Opaque,
/// with two constructors carrying different trust.
pub struct GuestArtifact(/* private */);

impl GuestArtifact {
    /// Raw component wasm, JIT-compiled at registration. Requires the `jit`
    /// feature; compilation runs on a blocking thread like `Source::load`.
    /// Safe: the bytes are validated and compiled inside the sandbox.
    pub const fn wasm(bytes: Vec<u8>) -> Self;

    /// A settings-matched pre-compiled artifact (`omnia compile` output);
    /// loaded via deserialization, no runtime codegen.
    ///
    /// # Safety
    /// A pre-compiled artifact is native code. Wasmtime's compatibility check
    /// (rejecting mismatched compile-affecting settings) is *not* an
    /// authenticity check — the caller attests the bytes are the unmodified
    /// output of a trusted build pipeline.
    pub const unsafe fn precompiled(bytes: Vec<u8>) -> Self;
}
```

The API lives on `Runtime` (not `Registry`) because serving a guest's linked exports needs the store factory (`dispatch/serve.rs` builds handlers from `Runtime::build_store`). Registration is async and eager: acquisition, verification, and compile cost are paid at install time, off every dispatch path — no dispatch ever waits on a fetch or a Cranelift run, so `guest_timeout` semantics are untouched.

Registration order (each step fails the whole call with no partial state): load the component → pre-instantiate (§4.2) → serve its linked exports (§4.3) → publish into the registry map. Upgrade is `deregister` + `register` (or a new digest-pinned id — identity is opaque, so a versioning scheme is the embedder's to choose).

### 4.2 Registry late insertion and removal (`crates/omnia/src/registry.rs`)

- `Registry` retains the shared `Linker` (today `assemble` consumes and drops it) and holds the guest map behind a concurrent-read, exclusive-write structure. `get` returns `Arc<Guest<T>>` (a cheap clone; `InstancePre` is itself internally shared) instead of `&Guest<T>` — the signature ripple across the trigger servers, `dispatch/host.rs`, and the testkit is mechanical.
- Pre-instantiation of a registered guest runs against a **per-registration clone** of the retained linker. This matters for imports: `dispatch::link` polyfills only the link-union interfaces some *static* guest imports (it takes the function types from the importing component). A registered guest importing an allow-listed interface no static guest imports gets that polyfill defined on the clone, from its own import types — the shared linker is never mutated after bootstrap, and existing `InstancePre`s are untouched. An import outside the linked host set and the `link` union still fails `instantiate_pre`, exactly as at bootstrap.
- `Registry::assemble` keeps rejecting an empty guest set for ordinary deployments, but `DeploymentBuilder::dynamic()` marks a deployment as dynamically populated, relaxing the check — a fully dynamic embedder starts empty and registers everything at run time.
- Static-route validation at `assemble` is unchanged and applies to static guests only: static `[[route.*]]` tables cannot name registered guests. Registered guests are reachable via host-mediated link dispatch, host→guest `Dispatcher::invoke`, and — for HTTP with an `http_fallback` installed — unrouted request paths (§4.5); per-trigger capability routing (HTTP/messaging/websocket `TriggerRouter`, and its CatchAll/Inert/ambiguity decisions) stays frozen at boot.

### 4.3 Serve-at-register (`crates/omnia/src/dispatch/serve.rs`, `transport.rs`)

Factor the per-guest body of `serve_links` (walk exports, filter by the link union, `serve_function` each, spawn drain tasks) into a helper callable for one guest. Registration calls it after `instantiate_pre` and adds the resulting `InProcServer` to the carrier's server map, which becomes concurrently updatable: the `DispatchHandle` transport stays install-once, and the map inside `InProcess` moves behind shared interior mutability (`InProcess` is `Clone`, so the map must be `Arc`-shared, not cloned by value). `deregister` removes the server entry; its drain tasks wind down with the last in-flight invocation.

Without this, a dispatch to a registered guest that exports a linked interface finds the registry entry but no wRPC endpoint — `InProcess::connect`'s "no in-process endpoint serves guest" error. The serve-before-publish ordering in §4.1 closes the inverse race.

### 4.4 Install-time compilation is the default acquisition pipeline

The recommended shape for a dynamic deployment is a two-step install, mirroring [embedded-guest](embedded-guest.md)'s build-time split but per component and per install:

1. The install step (plugin manager, OCI puller, directory watcher) fetches the raw `.wasm` and verifies it — digest, signature, provenance, whatever the deployment's policy demands.
2. It runs `omnia compile` to produce a settings-matched pre-compiled artifact, then calls `register` with the `unsafe` `GuestArtifact::precompiled` — the call site's attestation that the bytes came unmodified from its own trusted tooling.

The runtime then only deserializes: no Cranelift in the shipped binary (the `jit` feature stays optional), registration latency in milliseconds rather than seconds, and the compile-affecting-settings lockstep is contained to the install tooling. The trust story is the same as the static `.bin` path (which requires the same attestation via `DeploymentBuilder::precompiled()`'s unsafe `build`): the artifact is produced locally by trusted tooling from bytes the installer already verified. The safe `GuestArtifact::wasm` remains for `jit`-enabled deployments that prefer one fewer moving part.

One capacity note: the pooling allocator's totals (`pool_total_core_instances`, memories, tables) are fixed at engine build. A deployment that registers guests owns sizing that budget for its expected dynamic population.

### 4.5 Resolve-on-miss and trigger fallback

The lazy layer serves the two scenarios the push model cannot — cases where no host-side code observes the install, so nothing can call `register`:

1. **External traffic naming a component nothing local has acted on yet** (multi-tenant HTTP fan-out — a request for tenant X should fault in tenant X's component).
2. **A guest-side installer**: the component is written by a guest through a writable store mount mid-dispatch (a coordinator guest hydrating a dependency it is about to call). A guest cannot invoke a host registration API, and the embedder is blocked inside `run` — only a dispatch-triggered resolve can admit the component in the same process.

Two pieces, landing together:

- **`GuestResolver`** — a deployment-supplied `async fn resolve(&self, guest: GuestId, expected_export: String) -> Result<Option<GuestArtifact>>` (owned arguments, matching `Dispatcher::invoke` — the future outlives the borrow), configured through the programmatic `DeploymentBuilder` (a resolver is code, not manifest configuration; the standard `run(builder)` lifecycle offers no post-construction window for a setter). The signature gives each outcome its own structural slot instead of overloading an error type: `Ok(Some(artifact))` supplies the component; `Ok(None)` is the definitive miss — the resolver has no component for this identity, a valid answer rather than a fault (the registry's own lookup is `Option`-shaped for the same reason); `Err` means resolution *failed* (fetch error, failed digest verification) and the answer is unknown. The runtime consults it on a registry miss as a *thin adapter over `register`*: miss → single-flight per id (one in-flight resolution; every waiter awaits the same shared outcome, negatives included — N concurrent unknown-tenant requests cost one resolve, not N) → resolve → `register` → retry the lookup. Direct registration is untouched: a flight whose `register` loses to a concurrent embedder `register(id)` treats that as success after re-validating the winner, so the two paths need no shared lock and a resolver may even register ids itself. No negative outcome is cached across flights: `Ok(None)` and `Err` fail only the dispatches in that flight (fail closed; they differ only in error message and log severity), so a component installed between two calls becomes dispatchable on the next. `expected_export` — the dispatch site's required export interface, e.g. `wasi:http/handler` (version-tolerant) — is validated against the component type after load, before serve/publish (the resolver's answer is not trusted to be well-shaped; for a link target an unvalidated publish would create an entry whose endpoint every retry misses). Because it layers on `register`, all the §4.1–4.3 invariants (serve-before-publish, allow-list bounds, static-wins) hold for free.
- **Trigger fallback** (`http_fallback`, the same shape axum gives `Router::fallback`) — a deployment-supplied `fn(&RequestPath) -> Option<GuestId>` beside the static HTTP tables (also configured through `DeploymentBuilder`), consulted when no static prefix matches. Fallback identities go through the registry lookup and hence the miss hook; the HTTP server maps the outcomes faithfully — no fallback, a `None` fallback, or a resolver `Ok(None)` (unknown tenant) is a 404, while a resolver `Err` (acquisition/verification failure) is a 500. The trigger server then probes the fallback guest's typed handler indices *per request* (`ServiceIndices::new` is synchronous structural introspection, and probing against the exact `InstancePre` the request instantiates means indices can never skew across a deregister + re-register); a probe failure is the "resolver's answer is not well-shaped" error. The boot-built `TriggerRouter` is untouched — no shared mutable indices map — and static CatchAll/Inert/ambiguity decisions remain boot-frozen (a server with a fallback installed serves even when its static set is inert). Cache the probe in the trigger crate later only if measurement shows it matters.

Both pieces are additive; nothing in §4.1–4.4 anticipates them beyond the single-flight-friendly publish ordering.

### 4.6 Resolver-backed command guests

Command mode gains an explicit routing leg over the same lookup: `DeploymentBuilder::command_guest(id)` names the `wasi:cli/run` guest instead of relying on the sole-static-exporter catch-all, and `DeploymentBuilder::program_name(name)` overrides the deployment name used for telemetry and — in command mode — prepended to guest argv as `argv[0]` (defaulting, as before, to the manifest name). The command drive path then goes through the ordinary `Runtime::ensure_guest(id, "wasi:cli/run")`, so an explicit command guest participates in resolve-on-miss: a fully dynamic deployment starts with an empty registry and faults its command guest in through the resolver on the first (and only) miss. Fail-closed semantics replace the catch-all's inert exit `0` for this leg: an unresolved identity (no resolver, or a resolver `Ok(None)`), a resolver failure, and a resolved component lacking `wasi:cli/run` all fail the run, with the resolver's cause chain preserved through `EnsureError::ResolveFailed` for embedders that downcast to typed errors. A deployment that configures no explicit command guest keeps today's behavior exactly (sole static exporter, inert exit `0` when nothing exports `wasi:cli/run`).

## 5. What the runtime core never learns

The embedder owns everything domain-shaped: filesystem layout, filename ↔ id mapping, digest sidecar formats, registry clients, trust policy, id namespacing and versioning conventions. Two install-pipeline sketches that fit the same primitive:

- a **directory-store watcher**: new file under configured path globs → verify against the digest sidecar, fail closed → `omnia compile` → `register(id, precompiled)` — the plugin-host shape;
- an **OCI installer**: digest-pinned reference → pull, verify manifest digest → compile → `register` — the wasmCloud/Spin shape.

If a change to this design requires the core to parse an id, read a manifest convention, or know a directory layout, the change is wrong.

## 6. Alternatives considered

- **Resolve-on-miss as the primitive** (this design's previous draft). Lazily resolving inside dispatch demands single-flight, negative-caching policy, an `expected_export` threaded from every dispatch site, an async resolver seam on the hot path, and — the decisive flaw — a trigger point the guest→guest link path does not have: polyfilled imports go selector → `transport.connect` and never touch `Registry::get`, so the miss hook needs new plumbing to fire at all. The push primitive serves the motivating embedder scenario with none of that, and the resolver survives as a thin later layer (§4.5) for the cases that genuinely need laziness.
- **Regenerate the static manifest and restart.** Works, but makes every component acquisition a process boundary; unacceptable for an embedder that installs a component mid-run and calls it in the same process.
- **Pre-enumerate a superset of guests.** Pushes an unbounded inventory into every deployment and still fails for components that do not exist at boot.
- **Run dynamic components in separate omnia processes behind a distributed wRPC transport.** The transport seam (`dispatch/transport.rs`) anticipates exactly this, and it needs no registry mutation — but it changes the operational model (process per acquisition) and the per-call cost, and does not serve the single-process embedder.

## 7. Testing

- Seam tests (per the integration-first policy): register → link dispatch reaches the new guest; register → host→guest `Dispatcher::invoke` reaches it; deregister → dispatch fails as unregistered while an in-flight call completes; deregister + re-register swaps behavior (upgrade); register over a static id refused; deregister of a static id refused; a registered guest importing an interface outside the link union fails registration with no partial state; a `dynamic()` deployment starting with zero static guests.
- Extend `examples/guest-link`: a third guest absent from the manifest, verified and registered at run time from the target directory, reachable via link dispatch — proving serve-at-register end to end.
- Resolve-on-miss and trigger fallback tests (§4.5): single-flight observed once with every waiter sharing the outcome, neither a `None` nor a resolver error cached across flights, wrong-export component refused post-load, a direct `register` racing a flight, and the HTTP fallback faulting tenants in through one boot-frozen router.
- Resolver-backed command tests (§4.6): an empty dynamic registry resolving the explicit command guest on the first miss with exit codes passing through, resolver absence/decline/failure failing the run (cause chain preserved), a wrong-export component refused, `program_name` overriding `argv[0]`, and an explicit command guest naming a static entry hitting the registry without consulting the resolver.

## 8. References

- wasmCloud — components resolved and loaded by OCI identity on demand: <https://wasmcloud.com/docs/concepts/components/>
- Envoy — remote Wasm code with required SHA-256 before load (fail-closed verify-before-load): <https://www.envoyproxy.io/docs/envoy/latest/api-v3/extensions/wasm/v3/wasm.proto>
- Fermyon Spin — OCI registry–resolved applications: <https://spinframework.dev/v3/registry-tutorial>
- Kubernetes — digest-pinned images pulled at schedule time, not node start.
