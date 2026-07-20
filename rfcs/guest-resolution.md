# Design: Dynamic Guest Registration

> Status: Design proposal — lets an embedder grow (and shrink) the guest registry after startup through an explicit async registration primitive: verify → compile → pre-instantiate → serve → publish, at install time. Registration is the primitive; a lazy resolve-on-miss hook (`GuestResolver`) is a deferred thin layer over it, landing only with dynamic trigger projection. Complements — does not replace — the static `[[guest]]` population; a deployment that never registers behaves exactly as today. Depends: the guest registry (`crates/omnia/src/registry.rs`), host-mediated dispatch (`crates/omnia/src/dispatch/`), the programmatic `Manifest` / `DeploymentBuilder` (`crates/omnia/src/deployment/`), `omnia compile` (`crates/omnia/src/options/compile.rs`). Relates: [embedded-guest](embedded-guest.md) (the opposite trade: fully static composition at build time; this design grows composition at run time).

## 1. Motivation

The registry is frozen at assembly: `Registry::assemble` pre-instantiates every loaded guest into a `BTreeMap`, validates the route tables against it, and the only lookup thereafter is `Registry::get` → `Option`. A deployment that *acquires* a component after startup — an OCI pull, a package-manager install, a plugin dropped into a watched directory — cannot dispatch to it without a restart.

Growing a running host's component set is the standard shape for dynamic component platforms:

- **wasmCloud** references components by OCI identity and fetches, verifies, and instantiates them on demand across the lattice; the running host's component set is not fixed at boot. <https://wasmcloud.com/docs/concepts/components/>
- **Envoy** loads Wasm extensions from a remote data source at runtime and **requires a SHA-256 checksum** before the fetched module may be compiled — the same verify-before-load, fail-closed posture required here. <https://www.envoyproxy.io/docs/envoy/latest/api-v3/extensions/wasm/v3/wasm.proto>
- **Kubernetes** schedules workloads whose images are pulled on first use and pinned by digest; the node's runnable set grows after kubelet start.
- **Fermyon Spin** resolves application components from OCI registries at run, keyed by exact reference. <https://spinframework.dev/v3/registry-tutorial>
- Plugin hosts generally (VS Code activation events, Envoy filter chains) admit extension code while running rather than enumerating it at process start.

Omnia already treats guest identity as *data*: `GuestId` is an opaque ordered key the core never parses (`registry.rs` module doc), the dispatch selector reads the target id per call, and the manifest doc anticipates OCI as "another source kind". A registry that can grow while running is the natural completion of that stance; the frozen map is the anomaly.

The key observation shaping this design: in the motivating scenario — an embedder installs a component mid-run and calls it in the same process — **the embedder knows the moment the component exists**. It does not need the runtime to lazily discover the component inside a dispatch; it needs a way to hand the component to the runtime. A *push* (registration) primitive serves that directly and keeps the dispatch path untouched. The *pull* (resolve-on-miss) shape earns its extra machinery only when no host-side code observes the install: external traffic naming a component nothing local has acted on yet (dynamic HTTP fan-out), or an installer that is itself a *guest* — writing the component through a store mount mid-dispatch, with no host API to call and the embedder blocked inside `run`. Both are deferred to §4.5.

## 2. Current state: what exists, what is missing

What exists:

- **Identity-keyed dispatch.** Every path already resolves a `GuestId` per call: the dispatch selector for host-mediated links, the per-trigger `Router` for HTTP/messaging/websocket, the sole-exporter rule for CLI.
- **Programmatic deployment.** `Manifest` / `DeploymentBuilder` assemble the whole deployment in memory.
- **Loading machinery.** `Source::load` compiles or deserializes bytes; `Linker::instantiate_pre` against the shared linker is a per-guest operation; `omnia compile` already produces settings-matched pre-compiled artifacts.

What is missing — three structures freeze at bootstrap:

1. **The registry map.** `Registry::assemble` builds the `BTreeMap` once and drops the `Linker`; there is no insert or remove path and no retained linker to pre-instantiate against (`registry.rs`). `get` hands out `&Guest<T>` borrowed from the frozen map, which no concurrent-insert structure can do.
2. **The link serve side.** `serve_links` walks the registered guests once at startup, builds one `InProcServer` per exporting guest, and installs the `InProcess` carrier into a `OnceLock` (`dispatch/serve.rs`, `dispatch/handle.rs::install`). A guest inserted later that exports a linked interface has no server: `InProcess::connect` misses its map and the dispatch fails even though the guest is registered.
3. **Trigger routing.** `Router::build` validates every static route target against the registered set at bootstrap (`registry/routing.rs`); there is no deployment-supplied projection from a request key to an identity. (Deferred with resolve-on-miss, §4.5.)

## 3. Constraints

- **Law 2 holds.** The runtime core sees opaque guest ids and bytes. Where components live, how ids map to filenames or references, and what verification means are *deployment policy*: the embedder verifies (digest, signature, provenance) before calling `register`. The core never gains a store layout, a filename convention, or consumer vocabulary.
- **Fail closed.** The runtime registers exactly what it is handed and nothing else: no directory scan, no network probe, no fallback acquisition. An unregistered id is a dispatch error on the caller, same as today.
- **Static entries win.** `register` refuses an id that already names a static `[[guest]]` entry; `deregister` refuses a static entry. Only dynamically registered guests can be replaced or removed.
- **Registration cannot widen the allow-list.** The shared linker's host set and the deployment's `link` union are fixed at bootstrap. A registered guest may only import host interfaces already linked and only participate in link interfaces already in the `link` union. The allow-list stays a deployment declaration, not something a registered component can widen.
- **Serve before publish.** A registration is observable (present in the map, single-flight waiters released in §4.5) only after its link serve side is wired, so no caller can resolve the entry and then miss the wRPC endpoint.
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
/// deployment policy and happens before the runtime sees the bytes).
pub enum GuestArtifact {
    /// A settings-matched pre-compiled artifact (`omnia compile` output);
    /// loaded via deserialization, no runtime codegen.
    Precompiled(Vec<u8>),
    /// Raw component wasm, JIT-compiled at registration. Requires the `jit`
    /// feature; compilation runs on a blocking thread like `Source::load`.
    Wasm(Vec<u8>),
}
```

The API lives on `Runtime` (not `Registry`) because serving a guest's linked exports needs the store factory (`dispatch/serve.rs` builds handlers from `Runtime::build_store`). Registration is async and eager: acquisition, verification, and compile cost are paid at install time, off every dispatch path — no dispatch ever waits on a fetch or a Cranelift run, so `guest_timeout` semantics are untouched.

Registration order (each step fails the whole call with no partial state): load the component → pre-instantiate (§4.2) → serve its linked exports (§4.3) → publish into the registry map. Upgrade is `deregister` + `register` (or a new digest-pinned id — identity is opaque, so a versioning scheme is the embedder's to choose).

### 4.2 Registry late insertion and removal (`crates/omnia/src/registry.rs`)

- `Registry` retains the shared `Linker` (today `assemble` consumes and drops it) and holds the guest map behind a concurrent-read, exclusive-write structure. `get` returns `Arc<Guest<T>>` (a cheap clone; `InstancePre` is itself internally shared) instead of `&Guest<T>` — the signature ripple across the trigger servers, `dispatch/host.rs`, and the testkit is mechanical.
- Pre-instantiation of a registered guest runs against a **per-registration clone** of the retained linker. This matters for imports: `dispatch::link` polyfills only the link-union interfaces some *static* guest imports (it takes the function types from the importing component). A registered guest importing an allow-listed interface no static guest imports gets that polyfill defined on the clone, from its own import types — the shared linker is never mutated after bootstrap, and existing `InstancePre`s are untouched. An import outside the linked host set and the `link` union still fails `instantiate_pre`, exactly as at bootstrap.
- `Registry::assemble` keeps rejecting an empty guest set for ordinary deployments, but `DeploymentBuilder::dynamic()` marks a deployment as dynamically populated, relaxing the check — a fully dynamic embedder starts empty and registers everything at run time.
- Static-route validation at `assemble` is unchanged and applies to static guests only: static `[[route.*]]` tables cannot name registered guests. Registered guests are reachable via host-mediated link dispatch and host→guest `Dispatcher::invoke`; per-trigger capability routing (HTTP/messaging/websocket `TriggerRouter`, and its CatchAll/Inert/ambiguity decisions) stays frozen at boot until §4.5.

### 4.3 Serve-at-register (`crates/omnia/src/dispatch/serve.rs`, `transport.rs`)

Factor the per-guest body of `serve_links` (walk exports, filter by the link union, `serve_function` each, spawn drain tasks) into a helper callable for one guest. Registration calls it after `instantiate_pre` and adds the resulting `InProcServer` to the carrier's server map, which becomes concurrently updatable: the `DispatchHandle` transport stays install-once, and the map inside `InProcess` moves behind shared interior mutability (`InProcess` is `Clone`, so the map must be `Arc`-shared, not cloned by value). `deregister` removes the server entry; its drain tasks wind down with the last in-flight invocation.

Without this, a dispatch to a registered guest that exports a linked interface finds the registry entry but no wRPC endpoint — `InProcess::connect`'s "no in-process endpoint serves guest" error. The serve-before-publish ordering in §4.1 closes the inverse race.

### 4.4 Install-time compilation is the default acquisition pipeline

The recommended shape for a dynamic deployment is a two-step install, mirroring [embedded-guest](embedded-guest.md)'s build-time split but per component and per install:

1. The install step (plugin manager, OCI puller, directory watcher) fetches the raw `.wasm` and verifies it — digest, signature, provenance, whatever the deployment's policy demands.
2. It runs `omnia compile` to produce a settings-matched pre-compiled artifact, then calls `register` with `GuestArtifact::Precompiled`.

The runtime then only deserializes: no Cranelift in the shipped binary (the `jit` feature stays optional), registration latency in milliseconds rather than seconds, and the compile-affecting-settings lockstep is contained to the install tooling. The trust story is the same as today's `.bin` path: the artifact is produced locally by trusted tooling from bytes the installer already verified. `GuestArtifact::Wasm` remains for `jit`-enabled deployments that prefer one fewer moving part.

One capacity note: the pooling allocator's totals (`pool_total_core_instances`, memories, tables) are fixed at engine build. A deployment that registers guests owns sizing that budget for its expected dynamic population.

### 4.5 Deferred: resolve-on-miss and trigger projection

The lazy layer lands later, for the two scenarios the push model cannot serve — cases where no host-side code observes the install, so nothing can call `register`:

1. **External traffic naming a component nothing local has acted on yet** (multi-tenant HTTP fan-out — a request for tenant X should fault in tenant X's component).
2. **A guest-side installer**: the component is written by a guest through a writable store mount mid-dispatch (a coordinator guest hydrating a dependency it is about to call). A guest cannot invoke a host registration API, and the embedder is blocked inside `run` — only a dispatch-triggered resolve can admit the component in the same process.

Two pieces, landing together:

- **`GuestResolver`** — a deployment-supplied `async fn resolve(&self, guest: &GuestId, expected_export: &str) -> Result<GuestArtifact, ResolveRefused>` (programmatic registration only; a resolver is code, not configuration). It is consulted on a registry miss and implemented as a *thin adapter over `register`*: miss → single-flight per id (losers await the winner's published entry) → resolve → `register` → retry the lookup. Refusals are not cached (a component installed between two calls becomes dispatchable on the next), and `expected_export` — the dispatch site's required export interface, e.g. `wasi:http/incoming-handler` — is validated against the component type after load; the resolver's answer is not trusted to be well-shaped. Because it layers on `register`, all the §4.1–4.3 invariants (serve-before-publish, allow-list bounds, static-wins) hold for free.
- **Trigger projection** — a deployment-supplied `fn(&RequestPath) -> Option<GuestId>` beside the static HTTP tables, consulted when no static prefix matches. Projected identities go through the registry lookup and hence the miss hook. This is also where the trigger servers learn to probe typed handler indices for late guests (the `TriggerRouter` indices map gains an insert path); static CatchAll/Inert/ambiguity decisions remain boot-frozen.

Both pieces are additive; nothing in §4.1–4.4 anticipates them beyond the single-flight-friendly publish ordering.

## 5. What the runtime core never learns

The embedder owns everything domain-shaped: filesystem layout, filename ↔ id mapping, digest sidecar formats, registry clients, trust policy, id namespacing and versioning conventions. Two install-pipeline sketches that fit the same primitive:

- a **directory-store watcher**: new file under configured path globs → verify against the digest sidecar, fail closed → `omnia compile` → `register(id, Precompiled)` — the plugin-host shape;
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
- Resolve-on-miss and trigger projection tests join with §4.5 (single-flight observed once, refusal not cached, wrong-export component refused post-load).

## 8. References

- wasmCloud — components resolved and loaded by OCI identity on demand: <https://wasmcloud.com/docs/concepts/components/>
- Envoy — remote Wasm code with required SHA-256 before load (fail-closed verify-before-load): <https://www.envoyproxy.io/docs/envoy/latest/api-v3/extensions/wasm/v3/wasm.proto>
- Fermyon Spin — OCI registry–resolved applications: <https://spinframework.dev/v3/registry-tutorial>
- Kubernetes — digest-pinned images pulled at schedule time, not node start.
