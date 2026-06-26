# Design: Unified Host→Guest Invocation

> Status: **Draft** — for review. Converges the trigger-server instantiate/call blocks (`wasi-http`, `wasi-messaging`, `wasi-websocket`) and the Phase 2a `dispatch_to_guest` / `HostDispatch` path (`wasi-model` `resolve`) onto a small set of **composable floor helpers** — not a single closure-driven primitive (see §9 for why the earlier `GuestInvoke::run` shape was dropped). Related: [guest-registry.md](guest-registry.md), [architecture.md](architecture.md#guest-to-guest-interaction-host-mediated-dynamic-linking), [wasi-model.md](wasi-model.md) §7.2, [DECISIONS.md](DECISIONS.md) Phase 2a.

## 0. Problem

Host-initiated calls into a guest export today follow **three divergent shapes**:

| Caller | Target selection | Instantiate + call | Depth bound | Timeout | Call style |
|--------|------------------|--------------------|-------------|---------|------------|
| HTTP / messaging / websocket servers | `TriggerRouter` + route key | duplicated per `server.rs` | no (top-level trigger) | per-server `guest_timeout` | typed `bindgen!` indices + `run_concurrent` |
| `resolve` (`wasi-model`) | `grants.references` → `GuestId` | `dispatch_to_guest` | yes (`DispatchHandle::enter`) | inherits outer `complete` timeout | dynamic `Val` + `call_async`, export found by name scan |
| Guest→guest `link` imports | `GuestSelector` | wRPC `serve_links` | yes | per-hop | wRPC carrier |

The trigger servers and `resolve` share the same invariants ([architecture.md](architecture.md)): **instance-per-call**, **no resource handles across the seam** (§4.5), and **per-call budgets** (`guest_timeout`, dispatch depth §6.6). They already share `Runtime::store` / `build_store` / `instantiate`. What diverges is everything around that: routing/probing, resource marshaling, the typed-vs-dynamic call, depth counting, and the concurrency/spawn policy.

Phase 2a closed the `resolve` gap with `dispatch_to_guest` and `HostDispatch`, but only `wasi-model` uses them. `resolve` is also the *most* divergent path: it scans the target's exports by name (`resolve_interface`) and calls dynamically (`Val` + `call_async`) where the trigger servers probe typed indices at build and call through `bindgen!`. That makes every new host→guest hop a design decision instead of a convention.

**Non-goal:** this RFC does **not** replace guest→guest wRPC linking (`WrpcState`, `link_dynamic`, `serve_links`). That path is guest-initiated through polyfilled imports; the helpers here are host-initiated into a known `GuestId`.

## 1. Goal

Make every **host-initiated** guest call share the same building blocks, so adding a new hop is composing known parts rather than inventing a lifecycle:

1. **One probe-at-build map** (`ExportIndexMap`) producing typed `bindgen!` indices, keyed by `GuestId` — used by trigger routing *and* `resolve`.
2. **One instantiate helper** on `Runtime` returning an *owned* `(Store, Instance)` (instance-per-call) — the caller keeps control of the call/await/spawn.
3. **One fire-and-forget trigger driver** (`trigger`) the two one-way triggers (messaging, websocket) share, since they are near-duplicates of each other.
4. **One nested-call helper** for effect hosts that need a depth-bounded hop spawned off the caller's guest loop (`resolve` today; future effect-host hops).
5. **Typed export calls everywhere** — `resolve` adopts a floor `references` WIT and typed indices, retiring the runtime name scan and dynamic `Val` path.
6. **A thin `GuestInvoker`** trait (type-erased `Runtime`) so host bindings that see only `StoreCtx` can reach the nested-call helper, replacing `HostDispatch`.

Backends (Kafka, genai, Redis, …) stay **transport/effect-only** — they implement `WasiMessagingCtx` / `WasiModelCtx` and never instantiate guests. Host→guest hops live in trigger servers and effect-host bindings (`complete` → `ToolHost` → floor).

**Design stance:** prefer **small orthogonal helpers each consumer composes** over a single primitive that owns the whole lifecycle through a caller-supplied closure. The shared mechanical part (`store` → `build_store` → `instantiate` → `indices.load`) is already four trivial `Runtime` calls; the value is in unifying *routing, the typed call, depth, and the nested spawn* — not in wrapping the four lines. The divergent control flow (HTTP response streaming, inline-await vs nested-spawn) stays explicit at each call site. See §9 for the rejected closure-driven alternative.

## 2. Proposed API (`omnia`)

### 2.1 `ExportIndexMap` — shared probe at registry build

`TriggerRouter::build` today probes every guest with a handler-specific `Indices::new` and pairs the map with a route `Resolver`. Split the probe loop out:

```rust
pub struct ExportIndexMap<I> {
    indices: HashMap<GuestId, I>,
}

impl<I> ExportIndexMap<I> {
    /// A guest is *capable* exactly when `probe` (a typed `Indices::new`) succeeds.
    pub fn build<T, E, F>(registry: &Registry<T>, mut probe: F) -> Self
    where F: FnMut(&InstancePre<T>) -> Result<I, E>;

    pub fn get(&self, id: &GuestId) -> Option<&I>;

    /// The capable identities (stable order), for `Router::build`.
    pub fn capable(&self) -> impl Iterator<Item = &GuestId>;
}
```

`TriggerRouter<I, R>` becomes `ExportIndexMap<I>` + `Router<R>` (routing logic unchanged; the duplicate probe loop in `TriggerRouter::build` collapses to `ExportIndexMap::build` + `Router::build` over `capable()`).

For **`resolve`**, build `ExportIndexMap<ReferencesIndices>` at registry assembly — no route table; lookup is the explicit `GuestId` from `grants.references`.

### 2.2 Floor WIT for `references` shelves

Replace runtime export scanning (`resolve_interface` in `dispatch.rs`) with the same probe pattern as HTTP/messaging. Add a **floor-owned** WIT package (Law 2 — not a consumer package name):

```wit
package omnia:references@0.1.0;

interface shelf {
  resolve: func(reference: string) -> list<u8>;
}
```

`bindgen!` in `omnia` → `ReferencesIndices`. A capable guest exports an interface compatible with this world (example shelf guests align their WIT). Probe at build; typed `call_resolve` at the call site. This is the change that makes `resolve` structurally identical to a trigger handler: typed indices, no name scan, no `Val`.

### 2.3 `Runtime::instantiate` — the instantiate helper

Collapse the duplicated `store` → `build_store` → `instantiate` sequence into one default method that returns *owned* values, leaving the call/await/spawn to the caller:

```rust
impl Runtime {
    /// Fresh store + instance for one host-initiated call (instance-per-call).
    /// The caller loads typed indices and decides how to run the export:
    /// await inline (messaging/websocket), move into a task (http streaming),
    /// or spawn nested (resolve).
    async fn instantiate(
        &self,
        instance_pre: &InstancePre<Self::StoreCtx>,
    ) -> Result<(Store<Self::StoreCtx>, Instance)>;
}
```

This is deliberately *not* a closure-taking driver. Returning the owned `Store` and `Instance` is what lets HTTP move them into its streaming task and `resolve` move them onto a spawned nested task, without the helper having to model either.

### 2.4 `trigger` — the fire-and-forget trigger driver

Messaging and websocket are not merely "both simple" — they are **near-duplicates of each other**. Strip `crates/wasi-messaging/src/host/server.rs` and `crates/wasi-websocket/src/host/server.rs` and the only differences are the inbound resource type and table accessor, the route resolution (topic vs route + catch-all), the generated handler accessor, and span/log names. The lifecycle tail — `instantiate` → `indices.load` → `run_concurrent` → `guest_timeout` — is identical. Collapse it into one driver these two share:

```rust
/// One-way (fire-and-forget) host→guest trigger: instantiate fresh, run the
/// caller's typed handler under `run_concurrent`, bounded by `guest_timeout`.
/// The caller's `body` loads typed indices, pushes the inbound resource, and
/// calls the export — returning `()` when the guest finishes.
pub async fn trigger<R, F, Fut>(
    runtime: &R,
    instance_pre: &InstancePre<R::StoreCtx>,
    body: F,
) -> Result<()>
where
    R: Runtime,
    F: FnOnce(&mut Store<R::StoreCtx>, Instance) -> Fut,
    Fut: Future<Output = Result<()>>;
```

Unlike the rejected universal driver (§9), a closure is acceptable *here* precisely because this path is one-way: no streaming result to deliver mid-execution, no nested-loop spawn, and the timeout wraps the whole call. `trigger` is the tier-2 helper for the two one-way triggers; HTTP (request/response + streaming body) does not use it (§4.2), and the effect-host path uses `spawn` (§2.5) instead.

### 2.5 `spawn` — the effect-host hop

The one genuinely tricky shared concern on the effect-host path is depth-bounding a hop and running it *off* the caller's guest loop. Wasmtime forbids recursive `StoreContextMut::run_concurrent` on the same thread, so a call made from inside a guest's loop (e.g. `complete` awaiting `resolve`) must spawn the callee; when the caller's loop parks awaiting the task, the ambient store clears and the callee runs unnested (Phase 2a decision in [DECISIONS.md](DECISIONS.md)).

Encapsulate exactly that — depth guard + spawn + fresh store — in one helper, rather than a general builder flag:

```rust
/// Depth-bounded host→guest call spawned off the caller's guest loop.
/// Enters `DispatchHandle::enter` (§6.6), then runs `body` on a spawned task
/// over a fresh instance from `instantiate`. Resource handles are rejected
/// on the way in and out (§4.5).
pub async fn spawn<R, I, F, Fut, T>(
    runtime: &R,
    target: &GuestId,
    indices: &ExportIndexMap<I>,
    body: F,
) -> Result<T>
where
    R: Runtime,
    I: Clone + Send + 'static,
    F: FnOnce(Store<R::StoreCtx>, Instance, I) -> Fut + Send + 'static,
    Fut: Future<Output = Result<T>> + Send,
    T: Send + 'static;
```

`spawn` is the refactor of `dispatch_to_guest` (`dispatch.rs:327`): same `enter`/`DepthGuard` and `contains_resource` rejection, but it instantiates via `instantiate` and calls through typed `body` instead of scanning exports and encoding `Val`s.

### 2.6 `GuestInvoker` — type erasure for store context

Host bindings (`complete`, …) see `StoreCtx`, not `Runtime`. Replace `HostDispatch` with a thin trait implemented blanket on `Runtime`, carrying the typed `references` shape rather than a generic dynamic call:

```rust
pub trait GuestInvoker: Send + Sync + 'static {
    /// Host-mediated `resolve` into a guest's `references` shelf, depth-bounded
    /// and instance-per-call. Wraps `spawn` + typed `call_resolve`.
    fn resolve_reference(
        &self,
        target: GuestId,
        reference: String,
    ) -> FutureResult<Vec<u8>>;
}
```

`runtime!` threads `Arc<dyn GuestInvoker>` into `StoreCtx` (same slot as today's `host_dispatch`). The blanket `impl<R: Runtime> GuestInvoker for R` reaches the registry's `ExportIndexMap<ReferencesIndices>` and calls `spawn`.

**Delete** public `dispatch_to_guest`, `HostDispatch`, and `resolve_interface` once migration completes. No general dynamic `invoke(interface, func, Val…)` escape hatch is introduced: every host→guest hop gets floor WIT + typed indices (see §7 Q3).

## 3. Concurrency policy

| Context | Helper | Depth bound | Spawn | Notes |
|---------|--------|-------------|-------|-------|
| HTTP server | `instantiate` (+ caller's own task) | no | yes (streaming body outlives response) | Timeout is on *first response*, not the whole run; body keepalive stays in `server.rs` |
| messaging / websocket server | `trigger` (one shared driver) | no | no (per-message task is the server loop's, not the call's) | Simple await + `guest_timeout`; one-way, no result |
| `resolve` during `complete` | `spawn` | yes | yes (off caller's guest loop) | Caller awaits inside guest `run_concurrent`; depth-counted |
| future effect-host hop | `spawn` | yes | yes | Same as `resolve` |

The spawn policy is **not** a runtime flag: a top-level trigger composes `instantiate` directly, and a nested effect-host hop composes `spawn`. Choosing the wrong one is a type/composition difference at the call site, not an unchecked boolean.

## 4. Consumer migration

### 4.1 Trigger servers (messaging / websocket — one shared path)

**Before** (duplicated in *each* `server.rs`): `store()` → `build_store` → `instantiate` → `indices.load` → `run_concurrent` → `timeout`. Messaging and websocket carry near-identical copies of this tail.

**After** — both collapse to the same `trigger` call, varying only the resource push and the typed handler:

```rust
// messaging
trigger(&state, guest.instance_pre(), |store, instance| async move {
    let messaging = indices.load(store, &instance)?;
    messaging.wasi_messaging_incoming_handler()
        .call_handle(store, msg_res)
        .await
        .map(|_| ())
        .map_err(anyhow::Error::from)
})
.await?;

// websocket — same driver, different handler accessor + resource
trigger(&state, guest.instance_pre(), |store, instance| async move {
    let websocket = indices.load(store, &instance)?;
    websocket.omnia_websocket_handler()
        .call_handle(store, event_res)
        .await
        .map(|_| ())
        .map_err(anyhow::Error::from)
})
.await?;
```

The resource push (`table.push(message)` / `table.push(event)`) stays at the call site — it is store-ctx specific and precedes the store `trigger` builds. (Minor: `trigger` / `instantiate` may take an optional pre-built store-data, or expose `build_store` separately, so the resource can be pushed before instantiation — see §7 Q1.) Kafka and other messaging backends are unchanged.

### 4.2 HTTP server (the streaming exception)

HTTP keeps its outer `tokio::spawn` + `oneshot` + body-done keepalive: the response body must outlive `run_concurrent`, and the timeout is on time-to-first-response, not the whole execution. HTTP reuses only `instantiate`; the streaming control flow is a **documented, intentional exception**, not forced through a shared driver. This is an explicit non-goal of unification, not an open question.

### 4.3 `wasi-model` / genai

`BoundToolHost` holds `Arc<dyn GuestInvoker>` (from the store ctx). `ToolHost::resolve` → `GuestInvoker::resolve_reference` → `spawn` + typed `call_resolve`. The genai backend is unchanged — it calls `tool_host.resolve`; it never instantiates guests. The registry's `ExportIndexMap<ReferencesIndices>` is built once at assembly (§4.4) and reached by the blanket `GuestInvoker` impl.

### 4.4 Registry assembly

Extend registry build with the shared map alongside `Routes`:

- `resolve_targets: ExportIndexMap<ReferencesIndices>`
- Existing trigger routers rebuilt as `ExportIndexMap<I>` + `Router<R>` (the probe loop in `TriggerRouter::build` becomes `ExportIndexMap::build`).

## 5. Migration order

1. Extract `ExportIndexMap` from `TriggerRouter`; rebuild the three trigger routers on it (no behaviour change). Smallest, lowest-risk step.
2. Add `Runtime::instantiate` and the `trigger` driver; refactor **messaging** then **websocket** servers onto the shared `trigger` path.
3. Refactor **http** server to `instantiate`, keeping its streaming task/oneshot (it does not use `trigger` — §4.2).
4. Add floor `references.wit` + `ReferencesIndices`; build `ExportIndexMap<ReferencesIndices>` at registry assembly.
5. Add `spawn`; refactor `resolve` to it + typed `call_resolve`. Rename `HostDispatch` → `GuestInvoker` (typed `resolve_reference`).
6. Remove `dispatch_to_guest`, `HostDispatch`, and `resolve_interface` scanning.

Each step keeps acceptance tests green (`resolve_dispatches_to_a_fresh_shelf_per_call`, trigger integration tests, examples). Steps 1–3 are pure refactors; 4–6 land the typed `resolve`.

## 6. Acceptance criteria

- Every host-initiated guest call composes the shared helpers: the two one-way triggers (messaging, websocket) share `trigger`; HTTP reuses `instantiate` (+ its streaming tail); effect-host hops add `spawn`. All trigger routing goes through `ExportIndexMap`.
- `resolve` uses typed `ReferencesIndices` + probe-at-build, not runtime export scan or dynamic `Val`.
- Instance-per-call and dispatch-depth behaviour unchanged from Phase 2a tests.
- HTTP streaming remains correct (response body outlives `run_concurrent`); its exception is documented at the call site.
- No new host→guest pattern introduced in `wasi-*` crates without extending this RFC.
- `cargo make ci` green; all examples unchanged for single-guest deployments.

## 7. Open questions (for review)

1. **Resource push ordering** — messaging/websocket push a resource into the store-ctx table *before* instantiation. Does `instantiate` take optional pre-built store-data, or do we keep `build_store` public and have `instantiate` accept a `Store` so the push happens between `build_store` and instantiate?
2. **Registry surface** — expose `resolve_targets` on `Registry` vs a dedicated `InvokeTables` struct hung off the registry?
3. **Example shelf WIT** — re-export floor `omnia:references/shelf` vs duplicate a compatible interface in `examples/model/wit`?
4. **`GuestInvoker` surface** — keep it to typed `resolve_reference` only (current stance, no dynamic escape hatch), or anticipate a second typed effect-host hop now and generalise the trait shape before the second consumer exists?

## 8. References

- `crates/omnia/src/dispatch.rs` — `dispatch_to_guest`, `HostDispatch`, `resolve_interface` (to be replaced by §2.5/§2.6 + floor WIT)
- `crates/omnia/src/routing.rs` — `TriggerRouter` (source of `ExportIndexMap`)
- `crates/omnia/src/traits.rs` — `Runtime::{store, build_store, instantiate}` (gains `instantiate`)
- `crates/wasi-messaging/src/host/server.rs` — simple trigger pattern template
- `crates/wasi-http/src/host/server.rs` — the streaming exception (§4.2)
- `crates/wasi-model/src/host/model_impl.rs` — `BoundToolHost` / `resolve`
- `examples/model/wit/world.wit` — example `references` shelf

## 9. Rejected alternative: a single `GuestInvoke::run(closure)` primitive

An earlier draft proposed one primitive owning the whole lifecycle through a caller-supplied closure:

```rust
GuestInvoke::new(&state, guest_id)
    .top_level()                 // or .nested_in_guest_loop()
    .timeout(d)
    .run(|mut session| async move {
        let handler = indices.load(&mut session.store, &session.instance)?;
        session.concurrent(async |store| { /* call_handle */ }).await
    })
    .await?;
```

It was dropped because it abstracts the cheap, already-shared part and cannot absorb the parts that actually diverge:

- **It wraps four trivial lines.** `store` → `build_store` → `instantiate` → `indices.load` are already one-liners on `Runtime`; a builder + closure + `GuestSession::concurrent` is *more* indirection for the simple servers, not less.
- **HTTP can't fit.** HTTP moves the store into a spawned task, delivers the response over a `oneshot`, and keeps `run_concurrent` alive *after* the response (streaming body); its timeout is on first response, not the whole body. A `run(closure)` that owns the store and wraps the body in one timeout cannot express this — the caller has to re-introduce the exact spawn/oneshot machinery outside `run`, so the primitive abstracts the easy 20% and excludes the hard 80%.
- **`top_level` vs `nested_in_guest_loop` is a footgun.** The two always co-vary with depth counting, and the wrong combination is a runtime deadlock/panic, not a compile error. Composition (`instantiate` vs `spawn`) makes the choice a type/structure difference instead of an unchecked builder flag.

**Scope of this rejection.** What was rejected is a *universal* lifecycle owner that every host→guest hop routes through. A *scoped* closure driver is still the right tool where the control flow is genuinely uniform: `trigger` (§2.4) hands a closure the store + instance for the two one-way triggers. That is sound precisely because the fire-and-forget path has no streaming result, no nested-loop spawn, and a timeout that wraps the whole call — none of the three failure modes above apply. HTTP and the effect-host path stay out of it.

The composition approach (§2) keeps the same wins — one probe map, typed `resolve`, one fire-and-forget driver, one depth-bounded nested helper, `HostDispatch` retired — while leaving the genuinely different control flow explicit at each call site.
