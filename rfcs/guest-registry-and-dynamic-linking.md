# Design: The Guest Registry & Host-Mediated Dynamic Linking

> Status: **Working draft for collaboration.** This is the Omnia-side implementation sketch for the
> "Guest-to-guest interaction: host-mediated dynamic linking" section of [architecture.md](architecture.md),
> coordinating with [RFC-56](rfc-56-runtime-move.md). It is deliberately a living document — we refine it
> together until we are confident enough to start cutting code.

## 0. What we are building (and why it is two things)

The architecture sketch bundles two related-but-separable capabilities. We treat them as **two layers**
because the first is independently valuable and the second is built on top of it.

1. **The guest registry** — today Omnia loads exactly one component per process (`omnia run <guest>.wasm`).
   We want one Omnia process to hold *many* guests at once on a single `Engine` + `Linker`, each
   pre-instantiated and selectable by an opaque **identity**, instantiated fresh-per-call and discarded.
   This is pure infrastructure: it has value even before any guest calls another (it is what lets one
   binary route an HTTP request, a CLI command, and a topic message to *different* guests).

2. **Host-mediated dynamic linking** — a guest reaches another guest not by ahead-of-time composition but
   by importing an interface whose implementation the host satisfies at runtime: the host reads a
   **selector** from the call, looks the target guest up in the registry, instantiates it fresh, invokes
   the matching export, and returns the typed result. The carrier is a **pluggable transport** (in-process
   today; [wRPC](https://github.com/bytecodealliance/wrpc) across nodes later).

Keeping these layered matters for sequencing: we can land, test, and ship Layer 1 with zero new
third-party dependencies, then build Layer 2 against a stable foundation.

## 1. Goals and non-goals

### Goals

- A **generic, domain-agnostic** registry and linking mechanism that lives in the Omnia floor. The floor
  knows *opaque identities* and *the mechanism* — never `source`/`target`/`workflow` or any Specify
  concept (Law 2 in [architecture.md](architecture.md#the-four-laws)).
- **Instance-per-call** preserved everywhere, including dispatched calls — so a dispatched call lands in a
  fresh instance and can never *recursively* re-enter its caller.
- **Strict WIT typing across the seam** with no hand-rolled byte (de)serialization in guest code.
- A **transport seam** so "desktop → cloud" is a transport swap, not a code change. The in-process
  fast-path is a first-class transport, not a fallback.
- Backward compatibility: every current example (`examples/http`, `examples/messaging`, …) keeps working —
  the single-guest case becomes "a registry with one default entry".

### Non-goals (for this work)

- The `augentic:specify` WIT package (`source` / `target` / `references`) and the Specify guests — those
  live in the Specify consumer, built *on* this mechanism.
- The `wasi-model` `eval`/`resolve` callback ([RFC-53](rfc-53-wasi-model.md)/[RFC-59](rfc-59-model-tool-loop.md)).
  It is the *same* mechanism applied by the model backend; we make sure our primitive is general enough to
  serve it, but we do not build the model host here.
- OCI distribution of guests beyond a stubbed-out acquisition trait (we design the seam, defer the puller).

## 2. Where this lands in the current code

Today the relevant flow is (all in `crates/omnia` + `crates/runtime-macro`):

- `omnia::create(&wasm)` builds one `Engine`, one `Linker<T>`, loads one `Component`, returns `Compiled<T>`
  (`crates/omnia/src/create.rs`).
- The `omnia::runtime! { hosts: { … } }` macro generates a `Context` that implements `State`
  (`crates/runtime-macro/src/expand.rs`). `State` exposes exactly one `instance_pre()` and the
  per-call `store()` / `build_store()` / `instantiate()` helpers (`crates/omnia/src/traits.rs`).
- Trigger servers (`Server::run`) instantiate that single guest per request — e.g. the HTTP server in
  `crates/wasi-http/src/host/server.rs` calls `state.instance_pre()` → `build_store` → `instantiate`.

```77:107:crates/omnia/src/create.rs
pub struct Compiled<T: WasiView + 'static> {
    component: Component,
    linker: Linker<T>,
    options: RuntimeOptions,
}

impl<T: WasiView> Compiled<T> {
    // ...
    pub fn pre_instantiate(&self) -> Result<InstancePre<T>> {
        self.linker.instantiate_pre(&self.component).map_err(anyhow::Error::from)
    }
}
```

The shape of the change: **`Compiled` and `State` stop owning one `InstancePre` and start owning a
`GuestRegistry`** (a map of identity → `InstancePre`), plus the dynamic-link interceptors registered on the
shared `Linker` *before* pre-instantiation. The trigger servers select an entry by identity instead of
reading "the" instance.

## 3. Layer 1 — The Guest Registry

### 3.1 Identity is opaque data

The floor must not encode `source:`/`target:` semantics. A guest identity is an opaque, ordered key:

```rust
/// Opaque guest identity. The floor treats it as a string key; consumers
/// (e.g. Specify) project their own scheme onto it (`source:typescript`, …).
pub struct GuestId(pub Arc<str>);
```

Specify maps `source:typescript` → `GuestId("source:typescript")`; Omnia never parses it. Selector and
route-table logic operate on these opaque keys only.

### 3.2 The registry

```rust
/// One Engine + one Linker; many pre-instantiated guests keyed by identity.
pub struct GuestRegistry<T> {
    engine: Engine,
    options: RuntimeOptions,
    guests: HashMap<GuestId, Guest<T>>,
}

pub struct Guest<T> {
    id: GuestId,
    instance_pre: InstancePre<T>,
    // cached export-lookup indices, capabilities (e.g. exports wasi:http/incoming-handler?), etc.
}
```

Invariants:

- **One `Engine`, one `Linker<T>`.** Every guest is pre-instantiated against the *same* linker, so they
  share one set of host interfaces and one pooling pool. This is load-bearing for the
  instance-per-call cost story (`InstancePre` resolves imports + type-checks ahead of time;
  `crates/omnia/src/runtime.rs` already drives the epoch + pool metrics off one engine).
- **Pre-instantiation happens once, at registration.** Per call we only `instantiate_async` on a fresh
  `Store`, exactly as today.
- The registry is `Clone`-cheap (it is behind an `Arc`), matching how `Context`/`State` is cloned into
  each connection handler today.

### 3.3 Acquisition (how guests enter the registry)

A small trait so the source of a guest is pluggable:

```rust
pub trait GuestSource {
    /// Produce the raw component bytes (and identity) to register.
    fn load(&self, engine: &Engine) -> impl Future<Output = Result<Vec<LoadedGuest>>> + Send;
}

pub struct LoadedGuest { pub id: GuestId, pub component: Component }
```

Three concrete sources, in priority order of when we need them:

1. **File / CLI** (now) — `omnia run <guest>.wasm` loads one component, derives its identity from the file
   stem (preserving today's behaviour), and registers it as the **default** guest. This keeps every
   existing example working with a one-entry registry.
2. **Embedded** (next) — first-party guests baked in via `include_bytes!` for offline, zero-skew startup.
   A registration list the host binary supplies at build time.
3. **OCI, digest-pinned** (stub now, fill later) — resolve adapters lazily from an OCI store into a local
   cache. We define `GuestSource` + a cache path now and leave the puller as a follow-up.

`create()` becomes: build engine + linker → add host interfaces (as today) → **register dynamic-link
interceptors** (Layer 2, §4) → load every source's components → `instantiate_pre` each → assemble
`GuestRegistry`.

### 3.4 Trigger routing — selecting a guest by identity

The four triggers from [RFC-56](rfc-56-runtime-move.md) all reduce to "resolve an identity, then dispatch":

- **CLI** — names its guest directly (the default registry entry).
- **HTTP** — no identity on the wire, so the host derives it. Start with a **declarative longest-prefix
  route table** (`/target/omnia/… → target:omnia`), the Spin model; leave room for programmatic routing
  (compute identity from path/host/headers, mapping held in `wasi:keyvalue`). Only guests that *export*
  `wasi:http/incoming-handler` are routable — the registry records this capability per guest at
  registration.
- **Messaging / WebSocket** — same: derive identity from subject/route, resolve, dispatch.

The change to `Server::run` (e.g. `crates/wasi-http/src/host/server.rs`): instead of one
`ServiceIndices::new(state.instance_pre())`, the handler resolves a `GuestId` per request and pulls the
matching `Guest` from the registry. For a one-entry registry this is a trivial "the default guest".

### 3.5 `State` trait evolution

`State` (`crates/omnia/src/traits.rs`) currently returns one `instance_pre()`. We generalize:

```rust
pub trait State: Clone + Send + Sync + 'static {
    type StoreCtx: Send + HasLimits;
    fn store(&self) -> Self::StoreCtx;
    fn options(&self) -> &RuntimeOptions;

    /// The multi-guest registry.
    fn registry(&self) -> &GuestRegistry<Self::StoreCtx>;

    /// Back-compat convenience: the default guest (CLI/file entry).
    fn instance_pre(&self) -> &InstancePre<Self::StoreCtx> {
        self.registry().default_guest().instance_pre()
    }

    // build_store / instantiate unchanged, but `instantiate` takes a selected guest.
}
```

Keeping `instance_pre()` as a defaulted convenience means existing servers compile unchanged during the
migration, and we move them to identity-based selection one at a time.

## 4. Layer 2 — Host-mediated dynamic linking

### 4.1 The shape of a dispatched call

A caller guest imports an interface (say `omnia:link/echo`) and calls `echo(id, msg)`. The host has, on the
shared `Linker`, *polyfilled* that import so that invoking it:

1. **Extracts a selector** (a `GuestId`) from the call — by default the first argument, but the strategy is
   pluggable (see §4.3).
2. **Looks the target up** in the registry by identity.
3. **Instantiates it fresh** on a new `Store` with a fresh `StoreCtx` (instance-per-call).
4. **Invokes the matching export** — the same interface + function name the caller imported — over the
   bound **transport**, carrying the typed arguments.
5. **Returns the typed result** to the caller and discards the callee instance.

Because step 3 is always a fresh instance on a new store, a dispatched call **cannot recursively re-enter
its caller** — the one trap the component model still has. This guarantee falls out of the design, it is
not bolted on.

### 4.2 The transport seam

This is the single most important abstraction in the whole design, and where we depart slightly (and
deliberately) from the letter of the RFC for sequencing reasons (see §6.1). The carrier is a trait:

```rust
/// Carries one typed invocation to a registry guest's matching export and back.
pub trait LinkTransport: Clone + Send + Sync + 'static {
    fn invoke(
        &self,
        target: GuestId,
        interface: &str,
        func: &str,
        params: Vec<Val>,       // dynamic, store-independent values
        result_tys: &[Type],
    ) -> impl Future<Output = Result<Vec<Val>>> + Send;
}
```

Two implementations:

- **`InProcessTransport` (build first).** Resolves `target` in the registry, `instantiate_async` on a fresh
  `Store`, resolves the export `Func` by `interface`+`func`, `call_async(params)`, `post_return_async`,
  returns the results, drops the store. No serialization at all — `Val`s are copied directly. This is the
  RFC's "native in-process fast-path", and it works on the pinned `wasmtime = 46` today with **zero new
  dependencies**.
- **`WrpcTransport` (build later).** Encodes the invocation with wRPC and rides a pluggable wRPC transport
  (in-process duplex, Unix-domain socket, NATS, QUIC). This is what makes guests distributable across
  nodes. Gated on resolving the version question in §6.1.

The dispatch interceptor on the linker is identical for both — it only ever talks to `LinkTransport`.

### 4.3 Selector strategy (keeping the floor generic)

The floor must not hardcode "the adapter-id is the first argument" — that is a Specify contract detail.
We expose a strategy:

```rust
pub trait GuestSelector: Send + Sync + 'static {
    fn select(&self, interface: &str, func: &str, params: &[Val]) -> Result<(GuestId, Vec<Val>)>;
}
```

It returns the chosen identity **and** the parameter list to forward (so a strategy can strip the id arg or
pass it through — the architecture text implies the adapter imports/exports the same interface, so the
default forwards the id through; we will confirm the exact contract with Specify). The default
implementation: "first param is a string identity".

### 4.4 Wiring the interceptor onto the shared `Linker`

Wasmtime does **not** support wiring one component's export to another's import directly — the host must be
the intermediary (confirmed in [wasmtime#9309](https://github.com/bytecodealliance/wasmtime/issues/9309)).
The dynamic mechanism is `LinkerInstance::func_new` (the "reflection" path): define the imported function by
name with a closure receiving `&[Val]` params and writing `&mut [Val]` results. wasmCloud's
`wrpc-runtime-wasmtime` does *exactly* this — its `link_instance` / `link_function` walk a component's
imported interface types and polyfill each function onto a `wrpc_transport::Invoke` client. We mirror that
structure but target our `LinkTransport`:

1. At registration, introspect the importing component's type
   (`component.component_type().imports(&engine)`), find the targeted interface(s).
2. For each function in a targeted interface, `linker.instance(iface)?.func_new(func, closure)`.
3. The closure: run the selector → `LinkTransport::invoke` → write results. Because the call is async
   (it instantiates and runs a guest), this uses the component-model concurrent path
   (`func_new` + the futures/`run_concurrent` machinery already used in
   `crates/wasi-http/src/host/server.rs`).

Which interfaces are "targeted" is **configuration** the host binary supplies (e.g. the macro lists them),
so the floor stays generic — it links whatever interfaces it is told to, by name.

### 4.5 Resources do not cross the seam

The architecture is explicit: plain records cross by value; a live resource (the working-tree
`descriptor`) never crosses — the serving node re-materializes its own tree from a content-addressed
`revision`/`changeset`. For the generic mechanism this is a clean, enforceable rule: the dispatch path
supports plain `Val`s and **rejects** a resource handle attempting to cross, with a typed error. This both
matches the contract and sidesteps the hardest part of cross-store/cross-node resource translation.

## 5. Proof example (the acceptance vehicle)

A new `examples/linking` with two tiny guests, proving the generic mechanism with no Specify concepts:

- `responder` — exports `omnia:link/echo` with `echo(msg: string) -> string`.
- `router` — imports `omnia:link/echo` and calls `echo(id, msg)`; triggered via CLI/HTTP.

The host registers both under identities `responder` and `router`, links `omnia:link/echo` as a
host-mediated interface with the default first-arg selector, and the `InProcessTransport`. Calling `router`
dispatches to `responder` in a fresh instance and returns the echoed string. This is the end-to-end
acceptance test for Layers 1 + 2.

## 6. Key decisions & open questions (let's resolve these together)

### 6.1 Transport-first ordering and the wRPC/wasmtime version gap *(needs your call)*

The RFC names wRPC as the universal carrier. There is a concrete blocker: `wrpc-runtime-wasmtime` (latest
`0.30.0`, Nov 2025) tracks **wasmtime ^38**, while Omnia pins **`wasmtime = 46.0.1`** (with security-patch
discipline noted in the root `Cargo.toml`). Adopting wRPC's wasmtime integration today would mean either
downgrading wasmtime (unacceptable — security pins) or carrying an incompatible dependency.

**Proposed resolution (my recommendation):** lead with `InProcessTransport` behind the `LinkTransport`
seam. It delivers the *full* multi-guest + host-mediated-linking semantics on wasmtime 46 with no new
deps, and it is the exact "native in-process fast-path" the RFC says to keep in reserve. Add
`WrpcTransport` when (a) `wrpc-runtime-wasmtime` supports wasmtime 46, or (b) we decide to vendor the small
dynamic encode/decode surface we need. This *de-risks* the RFC's own "wRPC is a pre-1.0 dependency on the
hot path" concern rather than contradicting it. **Question: are you happy to sequence wRPC behind a working
in-process transport, given the version gap?**

### 6.2 Cross-repo boundary: where does the registry live? *(needs alignment)*

[RFC-56](rfc-56-runtime-move.md#the-cross-repo-boundary) attributes "the registry" to **Specify**, yet you
want the registry + dynamic linking to be a long-lived **Omnia** core. I think both are right under a
distinction:

- **Omnia owns the mechanism**: the `GuestRegistry` *type*, the `LinkTransport` seam, the dynamic
  interceptor, the selector trait, instance-per-call dispatch. All domain-agnostic.
- **Specify owns the population & policy**: *which* guests are registered, the `source:`/`target:` naming
  scheme, the typed interfaces, the bound concrete transport, the route table contents.

This keeps Law 2 intact and satisfies your "core part of Omnia" requirement. **Question: does this split
match your intent, or do you want the registry to stay thinner (just the
linking primitive) with population helpers living in the consumer?**

### 6.3 Typed vs. dynamic dispatch in the floor

The RFC prefers `wit-bindgen-wrpc`-*generated typed bindings* over the dynamic value-introspection path.
But the floor, by design, does **not** know the interface types (`source`/`target` are Specify's), so the
*generic* mechanism must be the **dynamic** (`Val`-based) path. Typed bindings are a consumer-side
optimization: Specify (which knows the interfaces) can register a typed `LinkTransport`/dispatcher for its
hot interfaces while the floor's dynamic path remains the universal fallback. **Question: agreed that the
Omnia floor ships the dynamic path, and typed dispatch is an opt-in consumers layer on?**

### 6.4 Async dispatch ergonomics

Dispatching from inside a host import means running a guest *within* a host call. This is the trickiest
mechanic and the first thing I want to prototype. wasmCloud does it through the same wasmtime APIs we have
(`func_new` + concurrent futures), and our HTTP server already uses `store.run_concurrent`. I expect this
to work but want a spike to confirm budgets/cancellation compose cleanly (see §6.5).

### 6.5 Budgets, recursion, observability

- The existing per-call wall-clock `guest_timeout`, epoch yielding, fuel, and memory limits
  (`crates/omnia/src/options.rs`, `traits.rs`) must wrap **dispatched** calls too. A dispatched call gets
  its own fresh store and therefore its own budgets — confirm this is the behaviour we want (likely yes).
- **Cycle/depth control**: instance-per-call prevents *recursive re-entrance*, but A→B→A (fresh each time)
  is still possible and could run unbounded. Add a dispatch-depth counter carried in `StoreCtx` and a
  configurable max depth.
- Emit metrics per dispatch (target identity, latency, transport) alongside the existing pool/instantiation
  metrics, so nested instantiation cost is visible for pool sizing.

## 7. Proposed phased plan

Each phase is independently shippable and keeps `cargo make ci` green.

- **Phase 0 — Decisions.** Resolve §6.1–6.3 with you. Record the outcomes (and the
  "resources never cross the seam", "in-process transport first" decisions) in a new
  `DECISIONS.md`, which the RFCs already reference but does not yet exist.
- **Phase 1 — Guest registry.** `GuestId`, `GuestRegistry`, `GuestSource` (file source first); refactor
  `Compiled`/`create`/the runtime macro and `State` to hold a registry with a default entry; migrate the
  HTTP server to identity selection; all existing examples stay green. *(No new deps.)*
- **Phase 1b — Inbound routing.** Longest-prefix HTTP route table + `wasi:http/incoming-handler`
  capability detection; embedded guest source.
- **Phase 2 — In-process dynamic linking.** `LinkTransport` + `InProcessTransport`, `GuestSelector`, the
  linker interceptor, resource-crossing rejection, depth/budget controls; `examples/linking` proof.
- **Phase 3 — wRPC transport.** `WrpcTransport` over in-process duplex + UDS + NATS, gated on §6.1; the
  desktop→cloud transport swap demonstrated.
- **Phase 4 — Hardening.** Typed-dispatch opt-in seam for consumers, richer metrics, fault injection /
  failure-mode tests, docs.

## 8. References

- [architecture.md](architecture.md) — §"Guest-to-guest interaction" and §"Many guests, selected by identity".
- [RFC-56](rfc-56-runtime-move.md) — the runtime move and multi-guest registry.
- [RFC-53](rfc-53-wasi-model.md) / [RFC-59](rfc-59-model-tool-loop.md) — `eval`/`resolve`, the same mechanism applied by the model backend.
- [wRPC](https://github.com/bytecodealliance/wrpc), [`wrpc-runtime-wasmtime`](https://docs.rs/wrpc-runtime-wasmtime) — the dynamic polyfill/serve approach we mirror.
- [wasmtime#9309](https://github.com/bytecodealliance/wasmtime/issues/9309) — confirmation that component-to-component linking must go through the host.
- wasmCloud [linking components](https://wasmcloud.com/docs/v1/concepts/linking-components/) — prior art for host-mediated runtime links over wRPC.
