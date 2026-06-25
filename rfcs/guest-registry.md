# Design: The Guest Registry & Host-Mediated Dynamic Linking

> Status: Implementation plan. The Omnia-side design for the "Guest-to-guest interaction: host-mediated
> dynamic linking" section of [architecture.md](architecture.md), coordinating with
> [RFC-56](rfc-56-runtime-move.md).

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
   the matching export, and returns the typed result. The carrier is
   [wRPC](https://github.com/bytecodealliance/wrpc) **from day one**, behind a thin transport seam: the
   *same* wRPC carrier rides an in-process byte pipe on one node and a Unix-domain socket / NATS / QUIC
   across a cluster, so "desktop → cloud" is a transport swap, not a code change.

Keeping these layered matters for sequencing: Layer 1 (the registry) is pure wasmtime infrastructure with
no new dependencies and is independently valuable; Layer 2 adds the wRPC carrier on top.

## 1. Goals and non-goals

### Goals

- A **generic, domain-agnostic** registry and linking mechanism that lives in the Omnia floor. The floor
knows *opaque identities* and *the mechanism* — never `source`/`target`/`workflow` or any Specify
concept (Law 2 in [architecture.md](architecture.md#the-four-laws)).
- **Instance-per-call** preserved everywhere, including dispatched calls — so a dispatched call lands in a
fresh instance and can never *recursively* re-enter its caller.
- **Strict WIT typing across the seam** with no hand-rolled byte (de)serialization in guest code.
- A **transport seam** so "desktop → cloud" is a transport swap, not a code change — with **wRPC as the
universal carrier from day one** ([RFC-56](rfc-56-runtime-move.md): wRPC is the carrier on *every* leg,
not just the cross-node one), and an in-process byte pipe as its co-located fast transport.
- **Manifest-driven deployment**: which guests load, which imports are host-mediated, how requests route,
and which transport carries them are a startup `omni.toml` (§3.7), not build-time macro state — so a
consumer can add a digest-pinned adapter without recompiling the host.
- Backward compatibility: every current example (`examples/http`, `examples/messaging`, …) keeps working —
the single-guest case becomes "a registry with one default entry", driven by the `omnia run <wasm>`
shorthand.

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
- The `omnia::runtime! { hosts: { … } }` macro generates a `Context` that implements the `State` trait
(`crates/runtime-macro/src/expand.rs`) — which this design renames to `Runtime` (§3.5) and refers to as
`Runtime` from here on. It exposes exactly one `instance_pre()` and the per-call `store()` /
`build_store()` / `instantiate()` helpers (`crates/omnia/src/traits.rs`).
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

The shape of the change: `**Compiled` and `Runtime` stop owning one `InstancePre` and start owning a
`Registry**` (a map of identity → `InstancePre`), plus the dynamic-link interceptors registered on the
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
pub struct Registry<T> {
    engine: Engine,
    options: RuntimeOptions,
    guests: HashMap<GuestId, Guest<T>>,
}

pub struct Guest<T> {
    id: GuestId,
    target: Target<T>,
    // cached export-lookup indices, capabilities (e.g. exports wasi:http/incoming-handler?), etc.
}

/// A registry entry resolves to a local instance or a remote wRPC endpoint.
/// Phase 1/2 only populate `Local`; `Remote` arrives with the cluster
/// transports (Phase 3) and is what makes the desktop->cloud swap a config
/// change rather than a code change (see §3.6).
enum Target<T> {
    Local(InstancePre<T>),
    Remote(/* bound wRPC endpoint */),
}
```

Invariants:

- **One `Engine`, one `Linker<T>`.** Every guest is pre-instantiated against the *same* linker, so they
share one set of host interfaces and one pooling pool. This is load-bearing for the
instance-per-call cost story (`InstancePre` resolves imports + type-checks ahead of time;
`crates/omnia/src/runtime.rs` already drives the epoch + pool metrics off one engine).
- **Pre-instantiation happens once, at registration.** Per call we only `instantiate_async` on a fresh
`Store`, exactly as today.
- The registry is `Clone`-cheap (it is behind an `Arc`), matching how `Context`/`Runtime` is cloned into
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

The deployment manifest's `source` field (§3.7) selects one per guest; each maps to a `GuestSource`
implementation:

1. **Embedded** — first-party core guests baked in via `include_bytes!` for offline, zero-skew startup.
  The binary carries a build-time `name → bytes` map (declared alongside `runtime!`); the manifest
   activates and routes them by name.
2. **File** — a local `.wasm` path. `omnia run <guest>.wasm` is the one-guest shorthand: load it, derive
  its identity from the file stem, register it as the **default** guest — preserving today's behaviour
   and keeping every example working with a one-entry registry.
3. **OCI, digest-pinned** — resolve adapters lazily from an OCI store into a local cache. We define
  `GuestSource` + the cache path now and land the puller as a follow-up.

`create()` becomes: load the manifest (§3.7) → build engine + linker → add host interfaces (as today) →
**register dynamic-link interceptors** for the manifest's `link` interfaces (Layer 2, §4) → resolve and
load every guest's `source` → `instantiate_pre` each → assemble the `Registry` and route tables.

### 3.4 Trigger routing — selecting a guest by identity

The four triggers from [RFC-56](rfc-56-runtime-move.md) all reduce to "resolve an identity, then dispatch":

- **CLI** — names its guest directly (the default registry entry).
- **HTTP** — no identity on the wire, so the host derives it from a **declarative longest-prefix route
table** (`/target/omnia/… → target:omnia`) built from the manifest's `[[route.http]]` entries (§3.7).
Programmatic routing (compute identity from path/host/headers, mapping held in `wasi:keyvalue`) is
deferred. Only guests that *export* `wasi:http/incoming-handler` are routable — the registry records this
capability per guest at registration.
- **Messaging / WebSocket** — same: derive identity from topic/route (manifest `[[route.messaging]]` /
`[[route.websocket]]`), resolve, dispatch.

The change to `Server::run` (e.g. `crates/wasi-http/src/host/server.rs`): instead of one
`ServiceIndices::new(state.instance_pre())`, the handler resolves a `GuestId` per request and pulls the
matching `Guest` from the registry. For a one-entry registry this is a trivial "the default guest".

**Capability-based default routing.** When no route is configured (§3.7), routing defaults **per handler
interface**, by how many registered guests *export* that handler — not by the total guest count. "Exports
the handler" is precisely whether the typed indices each server already builds from a guest succeed:
`ServiceIndices::new(..)` for HTTP, `MessagingRequestReplyIndices::new(..)` for messaging,
`DuplexIndices::new(..)` for websocket. For each trigger's handler interface, count its exporters:

- **0 exporters** — the trigger is inert; nothing answers it.
- **Exactly 1 exporter** — that guest is the catch-all for the trigger; no route needed. The whole trigger
fans into it.
- **2+ exporters** — explicit `[[route.<trigger>]]` entries are required to disambiguate; the collision is
scoped to *that* interface only.

So routing stays zero-config exactly as far as it is unambiguous. `omnia run <guest>.wasm` is the
degenerate case — one guest, every handler has a single exporter, so every trigger it can answer routes to
it. A two-guest deployment where one exports `wasi:http/incoming-handler` and the other the messaging
handler is *also* zero-config: each interface has a sole exporter, so HTTP auto-routes to the first and
messaging to the second. Only when two guests export the *same* handler must that trigger's routes be
declared. Two refinements keep this unsurprising:

- **Ambiguity fails fast.** If a trigger has 2+ exporters and no `[[route.<trigger>]]` entries, that is a
startup error ("trigger `http` has 2 capable guests (`a`, `b`) but no routes"), not a deferred runtime
  1. A request that simply matches no configured route still falls through to a normal 404 / unrouteable.
- **An explicit route suppresses the implicit catch-all for its trigger.** Declaring any
`[[route.<trigger>]]` makes that trigger fully route-driven even if only one guest exports the handler —
so a typo'd prefix surfaces as a 404 rather than silently hitting the sole guest. A guest that no route
names (and that is not a sole exporter) is then reachable only by host-mediated linking (§4).

### 3.5 The `Runtime` trait (renamed from `State`)

The trait the generated `Context` implements (`crates/omnia/src/traits.rs`) is renamed `State` →
`Runtime`. `State` was misleading: this trait is the long-lived, `Clone` handle that owns the registry,
options, and instantiation helpers — *not* per-`Store` state. The actual per-store state is its `StoreCtx`
associated type, so keeping the outer trait called `State` put two different "states" in one definition.
`Runtime` names what it is — the host runtime context every trigger server (`Server::run`) is handed to
resolve and instantiate a guest.

Today it returns one `instance_pre()`; we generalize it to hold the registry:

```rust
pub trait Runtime: Clone + Send + Sync + 'static {
    type StoreCtx: Send + HasLimits;
    fn store(&self) -> Self::StoreCtx;
    fn options(&self) -> &RuntimeOptions;

    /// The multi-guest registry.
    fn registry(&self) -> &Registry<Self::StoreCtx>;

    /// Temporary migration shim: the default guest (CLI/file entry).
    fn instance_pre(&self) -> &InstancePre<Self::StoreCtx> {
        self.registry().default_guest().instance_pre()
    }

    // build_store / instantiate unchanged, but `instantiate` takes a selected guest.
}
```

Omnia is pre-1.0, so the refactor breaks the trait — both the rename and the new registry method — cleanly
rather than carrying permanent shims (§6.4). The blast radius is mechanical: the `Server<S: State>` bound
becomes `Server<S: Runtime>`, every `wasi-*` server crate's `S: State` bound and the macro's generated
`use omnia::{… State}` import follow, with no behaviour change. `instance_pre()` is a temporary migration
aid — it lets the trigger servers and examples compile while they move to identity-based selection one at a
time — and is removed once the migration completes.

### 3.6 One registry, two callers (local vs. remote targets)

Inbound trigger routing and inter-guest communication are **not** separate mechanisms, and inter-guest
comms cannot be "just wRPC, no registry". The reason shapes the whole design.

A wRPC call has two halves: an **invoke** side (the caller's import, polyfilled onto a wRPC client) and a
**serve** side (the callee's export, served over the transport). Serving an export means *holding the
callee component and instantiating it per invocation*. That holder-and-instantiator **is** the registry.
So whether a call "goes through the registry" depends only on **where the target lives**:

```
Registry: GuestId -> Target { Local(InstancePre) | Remote(wRPC endpoint) }
wRPC          : the carrier on every leg

Inbound trigger (HTTP / messaging / ws): identity derived from the request       -> resolve -> serve
Inter-guest dispatch                   : identity from the call argument (adapter-id) -> resolve -> serve(local) | forward(remote)
```

The consequences:

- **Co-located guests (the single-binary desktop mode)** — to serve guest B's exports for a call from
guest A *in the same process*, the host must instantiate B from the identity→`InstancePre` map. There is
no "inter-guest comms with no registry" here; the registry is precisely the serve side of in-process
wRPC. This is the mode [architecture.md](architecture.md#many-guests-selected-by-identity) centers on
("the binary holds every guest on one runtime").
- **Remote guests (cluster mode)** — guest B runs as its own wRPC server on another process/node; A's
process does not hold B's `InstancePre`, only B's **endpoint**. But A still resolves the `adapter-id`
to that endpoint — an identity→endpoint lookup, i.e. a *distributed* registry. The registry concept is
unavoidable; only its *contents* change (local instances vs. remote endpoints), and that swap is exactly
what `Target::Remote` and a transport change express.

So the design is **one registry + wRPC as the carrier**, with two callers (inbound routing and inter-guest
dispatch) that differ only in *where the identity comes from* and *whether the target resolves local or
remote*. We deliberately do **not** build inter-guest as a second, registry-less path: that would either
duplicate the resolver or force a process-per-guest topology that sacrifices the offline, zero-skew,
single-binary CLI. Phase 1 builds the registry for inbound routing (where it is strictly required and
independently valuable); Phase 2's inter-guest dispatch reuses the same registry as the wRPC serve/resolve
layer.

### 3.7 Configuration: the deployment manifest

Registry population, routing, linking, and transport are **deployment** decisions, not build-time ones —
Specify resolves adapter guests dynamically (digest-pinned, from an OCI store), so which guests exist,
which of their imports are host-mediated, and how requests route are unknown when the binary is compiled.
They live in a startup **manifest** (`omni.toml`), loaded before the registry is built. The manifest is
**optional and sparse**: any field left out falls back to a synthesized default, and with no file at all
Omnia runs the single-guest zero-config default (§3.4). Only two things stay in the binary: the compiled
**host backends** (declared in `runtime!`) and the **bytes** of embedded core guests (`include_bytes!`,
referenced from the manifest by name).

The manifest is parsed **generically** — Omnia sees opaque `GuestId`s and interface *strings*, never
`source:`/`target:`/`mcp`. Specify writes the concrete file; the floor stays Law-2 clean.

```toml
# omni.toml — loaded at startup; defines the deployment, not the build

# --- Population: identity -> where the bytes come from -----------------------
[[guest]]
id = "workflow"
source.embedded = "workflow"          # a build-time include_bytes! blob, by name
link = ["augentic:specify/source", "augentic:specify/target"]   # host-mediated imports

[[guest]]
id = "target:omnia"
source.oci = "ghcr.io/augentic/target-omnia@sha256:abc…"        # digest-pinned

[[guest]]
id = "mcp"
source.path = "./guests/mcp.wasm"

# --- Inbound routing: orthogonal to population (a guest may have no route) ----
[[route.http]]
prefix = "/mcp"                        # longest-prefix wins
guest  = "mcp"

[[route.messaging]]
topic = "specify.build.>"
guest   = "workflow"

# --- Transport: how host-mediated calls travel ------------------------------
[transport]
default = "in-process"                 # in-process | unix | nats | quic
# [transport.target."target:omnia"]    # per-target override for distributed nodes
# kind = "nats"
# address = "nats://…"
```

The settled shape:

- **The manifest is optional; defaults fill every gap.** It is a sparse override layer, not the source of
truth: with no file, Omnia synthesizes a one-guest deployment (the CLI/embedded guest as default,
capability-gated catch-all routing to it, no links, in-process transport); a present manifest overrides
field-by-field. Routing defaults **per handler interface by exporter count** (§3.4) — a sole exporter is
the catch-all for its trigger, and explicit routes are required only where two or more guests export the
*same* handler.
- **Population and routing are separate sections.** A `[[guest]]` may carry no route — it is then reachable
only by the CLI trigger and host-mediated linking (§3.4) — so routing is not nested under the guest the
way Spin nests a component under a trigger.
- `**link` is per-guest.** Each guest names the imports the host should dispatch; the floor unions them
when wiring the shared linker (§4.4).
- **The selector ships with its default** ("first call argument is the identity", §4.3); making it
per-interface configurable in the manifest is a later refinement.
- **Transport is a global `default` plus optional per-target overrides** — this is the desktop→cloud swap
(§4.2), expressed as config.
- **Programmatic routing is deferred.** The static table is the floor; computing identity from
path/host/headers via `wasi:keyvalue` (the architecture's "ceiling") is designed when needed.
- **Engine tunables stay env-driven.** `RuntimeOptions` (`crates/omnia/src/options.rs`) keeps owning engine
and per-store settings; the manifest is purely *deployment shape*. The two surfaces do not overlap.

Startup order: load `omni.toml` **if present** (else use the synthesized default) → resolve every `source`
(embedded / file / OCI) → build the shared `Engine` + `Linker`, wiring the union of `link` interfaces as
host-mediated interceptors → pre-instantiate each guest → assemble the `Registry` and the route
tables. `omnia run <guest>.wasm` needs no manifest — it is "one guest, all supported triggers route to it,
no links" — while `omnia run --config omni.toml` (or the `OMNI_CONFIG` env var) drives a richer,
multi-guest deployment.

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

### 4.2 The transport seam — wRPC from day one

[wRPC](https://github.com/bytecodealliance/wrpc) is the carrier, full stop. Both ends use wRPC's own
wasmtime integration:

- **Imports** (caller side) are polyfilled onto a `wrpc_transport::Invoke` client — wRPC's
`wrpc-wasmtime` crate provides `link_instance` / `link_function`, which walk an imported interface's type
and define each function on a `LinkerInstance` so a call is encoded and sent over the wRPC transport.
- **Exports** (callee side) are served over a `wrpc_transport::Serve` handle — the host accepts an
invocation, resolves the target identity, instantiates the registry guest fresh, calls the matching
export, and streams the typed result back.

What is *pluggable* is the wRPC **transport**, not the RPC framework. We model that as a thin seam so the
same dispatch path runs co-located or distributed:

```rust
/// A bound wRPC transport: a client handle (Invoke) and, where this node also
/// serves guests, a server handle (Serve/Accept). The dispatch path only ever
/// talks to this — never to a specific transport implementation.
pub trait LinkTransport: Clone + Send + Sync + 'static {
    type Invoke: wrpc_transport::Invoke;
    fn client(&self) -> &Self::Invoke;
    // serving side wired separately at registration for the guests this node hosts
}
```

Transport implementations, in the order we need them:

- **In-process pipe (day one).** wRPC's frame transport over an in-memory `tokio::io::duplex` byte stream:
full wRPC encode/decode, zero network, both peers in one process. This is the co-located fast transport
and the manifest's `default` when no other is bound (§3.7). (If profiling ever shows the encode/decode is
on a hot path, the RFC's
"native in-process fast-path" — a direct `Instance::get_func` + `Func::call_async`, bypassing
serialization — stays available behind the same seam as an optimization. We do **not** build it first;
wRPC-in-process is the baseline.)
- **Unix-domain socket (day one / next).** `wrpc-transport`'s UDS `Client`/`UnixListener` — same node,
separate processes; the natural step between in-process and the network.
- **NATS / QUIC (cluster).** Distributed legs; wRPC ships both. This is purely a bound-transport change.

The dispatch interceptor on the linker is identical across all of them — it only ever talks to wRPC.

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
default forwards the id through; the exact contract is a Specify-side detail). The default implementation:
"first param is a string identity".

### 4.4 Wiring the interceptor onto the shared `Linker`

Wasmtime does **not** support wiring one component's export to another's import directly — the host must be
the intermediary (confirmed in [wasmtime#9309](https://github.com/bytecodealliance/wasmtime/issues/9309)).
The dynamic mechanism is `LinkerInstance::func_new` (the "reflection" path): define the imported function by
name with a closure receiving `&[Val]` params and writing `&mut [Val]` results. We do not hand-roll this —
wRPC's `wrpc-wasmtime` crate already provides exactly the right primitives, used by `wrpc-wasmtime` itself
and by wasmCloud:

1. At registration, introspect the importing component's type
  (`component.component_type().imports(&engine)`), find the targeted interface(s).
2. Polyfill each targeted interface onto the shared linker with `wrpc_wasmtime::link_instance` (which calls
  `link_function` / `func_new` under the hood), bound to our wRPC `Invoke` client.
3. Wrap the client so the **selector** (§4.3) picks the wRPC peer/target from the call before the
  invocation is encoded — this is the one place Omnia's identity resolution sits in the path.

Because the call is async (it crosses wRPC and ultimately instantiates and runs a guest), this rides the
component-model concurrent path — the same futures / `run_concurrent` machinery already used in
`crates/wasi-http/src/host/server.rs`.

Which interfaces are "targeted" is an **explicit allow-list declared per guest in the deployment
manifest** (§3.7), not in the `runtime!` macro — Specify resolves adapters dynamically (by OCI digest), so
the set is a runtime concern unknown at build time. The floor stays generic: it links whatever interfaces
the manifest names, by string, treating each as an opaque interface name (it never parses
`augentic:specify`). At startup, for each guest the host introspects its imports; an imported interface
listed in that guest's `link` is polyfilled per steps 1–3 above. We deliberately do **not** auto-link
unsatisfied imports: an import that is neither host-satisfied nor listed in any `link` is a startup
(pre-instantiation) error, which keeps the dispatched seam explicit and surfaced before traffic flows.

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
host-mediated interface with the default first-arg selector, and binds the **in-process wRPC transport**.
Calling `router` dispatches to `responder` over wRPC in a fresh instance and returns the echoed string —
the end-to-end acceptance test for Layers 1 + 2 over the real wRPC carrier. A follow-up variant binds the
**UDS** transport with no guest or dispatch-code change, proving the transport swap.

## 6. Design decisions and rationale

### 6.1 wRPC from day one, pinned to a git revision

wRPC is the carrier from day one. Its `main` workspace tracks Omnia's exact toolchain — `wasmtime = "46"`
and `wit-bindgen = "0.58"` — so there is no version skew. The wasmtime integration crate
(`wrpc-wasmtime 0.1.0`, at `crates/wasmtime`, the successor to the published `wrpc-runtime-wasmtime`) and
`wrpc-transport 0.29` are not yet released on crates.io for the wasmtime-46 line, so day one means pinning
to a specific git revision of `bytecodealliance/wrpc`:

```toml
# Cargo.toml (workspace) — pin a reviewed rev; bump deliberately, like the wasmtime pins.
wrpc-transport = { git = "https://github.com/bytecodealliance/wrpc", rev = "<sha>" }
wrpc-wasmtime  = { git = "https://github.com/bytecodealliance/wrpc", rev = "<sha>" }
```

The rev is held with the same discipline as the wasmtime security pins — bumped deliberately, reviewed on
bump — and re-pinned to crates.io once a wasmtime-46 line is published. Two constraints follow:

- `wrpc-wasmtime` pulls `wasmtime-wasi` with the **p2** feature; Omnia enables **p3**. Features are
additive so these coexist; the linker wiring (both `p2::add_to_linker_async` and `p3::add_to_linker` in
`crates/omnia/src/create.rs`) is verified in the spike.
- wRPC is **pre-1.0**. Its churn is contained behind the dispatch seam (the targeted imports) and never
reaches the typed contract or the guests — [RFC-56](rfc-56-runtime-move.md) records the same constraint.
The `LinkTransport` seam is that containment.

### 6.2 Omnia owns the mechanism; the consumer owns population and policy

[RFC-56](rfc-56-runtime-move.md#the-cross-repo-boundary) attributes "the registry" to Specify; here the
registry, routing, and dynamic linking are a long-lived Omnia core. Both hold under a mechanism/population
split:

- **Omnia owns the mechanism** — the `Registry` type, the routing (identity resolution, the HTTP
route-table machinery, trigger dispatch), the `LinkTransport` seam, the dynamic interceptor, the selector
trait, and instance-per-call dispatch. This is generic plumbing with subtle correctness requirements (one
shared `Engine`/`Linker`, pre-instantiation ordering, pool sizing) that no consumer should re-implement.
- **The consumer owns population and policy** — which guests it loads (the `include_bytes!` core list, the
OCI puller), the identity scheme it projects onto `GuestId` (e.g. Specify's `source:`/`target:`), the
typed interfaces, the bound transport, the per-guest link allow-list, and the route-table *contents* —
most of it expressed in the deployment manifest (§3.7), which the floor parses generically.

The route-table machinery is Omnia-generic (prefix → opaque `GuestId`); only its entries are consumer
config. The floor compiles knowing zero identities, interface names, or route contents, preserving Law 2.

### 6.3 The floor ships the dynamic path; typed dispatch is a consumer opt-in

The floor does not know the interface types (`source`/`target` are Specify's), so its generic mechanism is
the dynamic (`Val`-based) path that `wrpc-wasmtime`'s `link_instance` provides. Typed dispatch is a
consumer-side opt-in: Specify, which owns `augentic:specify`, may generate `wit-bindgen-wrpc` bindings and
register a typed `Invoke` through the same `LinkTransport` seam for its hot interfaces, gaining the
monomorphized win without the floor learning the types. Both ends emit the same wRPC wire encoding, so a
typed caller and a dynamically-served callee interoperate. The resources-don't-cross rule (§4.5) is
enforced generically on the dynamic path by rejecting a `Val::Resource` crossing the seam.

### 6.4 Breaking changes and the explicit link allow-list

Omnia is pre-1.0 (`0.35.0`), so the refactor breaks the public `Runtime` trait (renamed from `State`,
§3.5) and the `runtime!` macro surface cleanly rather than carrying permanent compatibility shims; the
`instance_pre()` aid in §3.5 is temporary and removed after migration.

Host-mediated interfaces are an explicit allow-list the consumer declares **per guest in the deployment
manifest** (§3.7, §4.4), not in the macro and not auto-linked — Specify loads adapter guests dynamically,
so the set is a runtime decision. An import that is neither host-satisfied nor listed in any guest's `link`
is a startup (pre-instantiation) error, which keeps the dispatched seam explicit and surfaced before
traffic flows. Auto-linking unsatisfied imports is out of scope (a possible future opt-in).

### 6.5 Async dispatch ergonomics

Dispatching from inside a host import runs a guest within a host call. wasmCloud does this through the same
wasmtime APIs Omnia uses (`func_new` + concurrent futures), and the HTTP server already uses
`store.run_concurrent`. The spike confirms this composes — in particular that budgets and cancellation
(§6.6) compose cleanly — before Phase 2 builds on it.

### 6.6 Budgets, recursion, observability

- The per-call wall-clock `guest_timeout`, epoch yielding, fuel, and memory limits
(`crates/omnia/src/options.rs`, `traits.rs`) wrap dispatched calls too: a dispatched call gets its own
fresh store and therefore its own budgets.
- Instance-per-call prevents *recursive* re-entrance, but A→B→A (fresh each time) can still run unbounded,
so a dispatch-depth counter carried in `StoreCtx` enforces a configurable maximum depth.
- Each dispatch emits metrics (target identity, latency, transport) alongside the existing
pool/instantiation metrics, so nested instantiation cost is visible for pool sizing.

## 7. Phased plan

Each phase is independently shippable and keeps `cargo make ci` green.

- **Phase 0 — `DECISIONS.md`.** Record the design decisions (resources never cross the seam, the wRPC
git-pin, the mechanism/population split, the dynamic-path floor, the breaking-change allowance, and the
manifest-driven per-guest link allow-list) in a new `DECISIONS.md`, which the RFCs already reference but
does not yet exist.
- **Phase 1 — Guest registry.** `GuestId`, `Registry`, `GuestSource` (file source first); the
`omni.toml` loader for population and transport, with `omnia run <wasm>` kept as the one-guest shorthand
(§3.7); refactor `Compiled`/`create`/the runtime macro and the `Runtime` trait (§3.5) to hold a registry
with a default entry; migrate every trigger server (`wasi-http`, `wasi-messaging`, `wasi-websocket`) to identity
selection; remove the temporary `instance_pre()` shim once migrated (§3.5); all existing examples stay
green. No new dependencies — independent of wRPC.
- **Phase 1b — Inbound routing.** Per-trigger route tables from the manifest's `[[route.*]]` entries
(longest-prefix for `[[route.http]]`, topic/route match for `[[route.messaging]]` and
`[[route.websocket]]`) + per-trigger handler-export capability detection (`ServiceIndices` for HTTP,
`MessagingRequestReplyIndices` for messaging, `DuplexIndices` for websocket) driving the
capability-based default routing of §3.4; embedded guest source. CLI needs no route table — it names its
guest directly (§3.4) and is handled by Phase 1's default-entry work.
- **Spike (gates Phase 2).** The wRPC git-pin builds clean against `wasmtime 46.0.1` (p2/p3 coexistence,
§6.1); a host import can instantiate and run another guest under the component-model concurrent model
(§6.5); the in-process wRPC `Invoke`/`Serve` wiring works. Throwaway branch, not shipped.
- **Phase 2 — Dynamic linking over in-process wRPC.** The per-guest manifest `link` allow-list (§3.7,
§4.4), the `LinkTransport` seam, the in-process wRPC transport, `wrpc-wasmtime` import polyfill + export serving,
`GuestSelector`, resource-crossing rejection, depth/budget controls; the `examples/linking` proof.
- **Phase 3 — Additional transports.** UDS (same node, separate processes), then NATS / QUIC for
cross-node; demonstrate the desktop→cloud transport swap with no guest or dispatch change.
- **Phase 4 — Hardening.** Optional native fast-path behind the seam (only if profiling demands), richer
metrics, fault injection / failure-mode tests, docs.

## 8. Implementation planning approach

This RFC is the design source of truth; it is **not** the execution plan. Turn it into work with a
**hybrid** of one durable index plus just-in-time per-phase plans — not a single monolithic plan, and not a
full set of per-phase plans written up front (phases 2+ are reshaped by the spike, so detailed plans for
them now would be speculation).

Three artifacts, each with one job:

1. **This RFC** — the *design*. Plans reference it; they do not duplicate or re-litigate it.
2. `**DECISIONS.md` + a thin master index** (Phase 0). The master index mirrors §7: the phase list, each
  phase's **exit criteria** and **dependencies**, and the cross-phase **invariants** every phase must
   preserve. `DECISIONS.md` holds the settled-choices half (the list in Phase 0). This is the durable
   tracking surface; it is updated as phases land.
3. **A detailed per-phase plan, authored immediately before that phase starts** — file-level changes, task
  ordering, and the concrete acceptance test. Write **Phase 0 + Phase 1 (+ 1b) now** (well-understood,
   dependency-free, pure wasmtime). **Defer the Phase 2+ detailed plans until the spike resolves the
   unknowns** — the spike's output is what unlocks a non-speculative Phase 2 plan.

Rules for whoever (human or agent) drafts the plans:

- **One plan per phase, scoped to a commit boundary.** Each phase is independently shippable and keeps `cargo make ci` green (§7); a phase — or a sub-step like 1 vs 1b — is the unit of both a plan and a locally reviewable commit. Phase 1 is large (it breaks the `Runtime` trait and the macro and migrates *every* trigger server), so its plan enumerates sub-steps that each land green.
- **The spike is a throwaway checklist, not a plan.** It lives on a branch that is not shipped; its job is
to answer the questions in §7's spike entry (p2/p3 coexistence and the git-pin building against
`wasmtime 46.0.1` per §6.1; a host import instantiating and running another guest under the concurrent
model per §6.5; the in-process wRPC `Invoke`/`Serve` wiring). Capture findings, then write the Phase 2
plan against them.
- **Every plan must preserve the invariants**, and state how it does so in its acceptance test:
instance-per-call everywhere (including dispatched calls); `cargo make ci` green and all existing
examples working; the floor stays generic (Law 2 — no `source`/`target`/Specify identity leaks into
Omnia); resources never cross the seam (§4.5); per-call budgets/limits and dispatch-depth bounds hold
(§6.6).

A per-phase plan should contain: scope and the §7 entry it implements; ordered sub-steps mapped to commits
or PRs; the files/crates each touches; the acceptance test (the observable green-CI outcome); the
invariants it must hold; and its dependencies / what it unlocks.

## 9. References

- [architecture.md](architecture.md) — §"Guest-to-guest interaction" and §"Many guests, selected by identity".
- [RFC-56](rfc-56-runtime-move.md) — the runtime move and multi-guest registry.
- [RFC-53](rfc-53-wasi-model.md) / [RFC-59](rfc-59-model-tool-loop.md) — `eval`/`resolve`, the same mechanism applied by the model backend.
- [wRPC](https://github.com/bytecodealliance/wrpc) — the carrier. Relevant crates on `main`: `wrpc-transport` (0.29), `wrpc-wasmtime` (0.1, the wasmtime polyfill/serve integration, successor to the published `wrpc-runtime-wasmtime`), `wrpc-introspect`; the workspace pins `wasmtime = 46` and `wit-bindgen = 0.58`, matching Omnia.
- [wasmtime#9309](https://github.com/bytecodealliance/wasmtime/issues/9309) — confirmation that component-to-component linking must go through the host.
- wasmCloud [linking components](https://wasmcloud.com/docs/v1/concepts/linking-components/) — prior art for host-mediated runtime links over wRPC.

