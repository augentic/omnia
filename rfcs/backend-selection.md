# Design: Deploy-Time Backend & Host Selection

> Status: Design proposal — introduces a *dynamic* runtime-composition mode in which one prebuilt binary chooses its WASI backends (and which triggers run) from `omnia.toml`, with no recompile. Complements — does not replace — the compile-time `runtime!` macro. Depends: the `runtime!` macro and the library `omnia::StoreCtx<B>` (with its per-host `HasXxx` accessor traits), the per-interface `WasiXxxCtx` backend traits, the `omnia.toml` manifest. Relates: [rfc-58-backend-router](rfc-58-backend-router.md) (per-call *model* routing — orthogonal), [wrpc-cluster](wrpc-cluster.md) (out-of-process backends — the future extension in §7.1).

## 1. Motivation

Today the host:backend pairing is baked into concrete types at compile time by the `runtime!` macro:

```rust
omnia::runtime!({
    hosts: {
        WasiHttp: HttpDefault,
        WasiKeyValue: KeyValueDefault,
        WasiSql: SqlDefault,
    }
});
```

Each deployment is therefore a bespoke crate, and "configure the runtime" means writing Rust and running a full Rust + `wasm32-wasip2` toolchain build. That is a lot to ask of an operator whose only goal is to *run guests* against, say, Redis instead of the in-memory default.

The goal of this design is to let an Omnia **user** pick, for a single shipped binary and without recompiling:

- which capabilities (WASI hosts) are active, and
- which backend implementation backs each capability,

via the existing `omnia.toml` deployment manifest. This mirrors the manifest's own stated philosophy — registry population, routing, and transport are already *deployment* decisions, not build-time ones (`crates/omnia/src/deployment/manifest.rs`). Backend choice is the same kind of decision.

## 2. Current state: what is compile-time, what is already dynamic

The `runtime!` machinery (`crates/host-macros/src/runtime/{codegen,parse}.rs`, the library `omnia::StoreCtx<B>` over the connected backend bundle `B`, and the per-host `HasXxx` accessor traits) generates, per backend:

- a field on the connected `Backends` bundle (`key_value_default: KeyValueDefault`),
- a `Backends::connect()` impl that calls `<KeyValueDefault as Backend>::connect()` at startup (run by the library `Runtime::new`),
- a per-invocation `Runtime::store()` that **clones** the bundle into each store,
- a bundle-side `HasXxx` accessor (generated directly by `runtime!`) that the host crate's blanket `WasiXxxView for StoreCtx<B>` reads.

The decisive observation is that the host *logic* is **already dynamic**. Every interface exposes an object-safe context trait, and the generated view hands the host a trait object — not the concrete backend:

```rust
// crates/wasi-keyvalue/src/host.rs
pub struct WasiKeyValueCtxView<'a> {
    pub ctx: &'a mut dyn WasiKeyValueCtx,   // <- trait object
    pub table: &'a mut ResourceTable,
}

pub trait WasiKeyValueCtx: Debug + Send + Sync + 'static {
    fn open_bucket(&self, identifier: String) -> FutureResult<Arc<dyn Bucket>>;
}
```

Both the in-memory default and a real backend implement that one trait (the doc comment literally says "an in-memory store, or a Redis-backed store"). So the only pieces fixed at compile time are:

1. the **concrete type** of each `Backends` bundle field,
2. the **`Backend::connect()` wiring** in the generated `Backends::connect`,
3. the per-store **`.clone()`** of that concrete backend.

Connection settings are *already* runtime-resolved: `Backend::connect()` defaults to `Self::ConnectOptions::from_env()` (`crates/omnia/src/host.rs`). What is missing is selecting the *implementation* at runtime, not configuring it.

## 3. The hard constraint

A binary can only use backends **compiled into it**. The real backends (Redis, MongoDb, Postgres, NATS, …) live in the separate `backends` repo, which *depends on* `omnia`. Compiling them into a binary that ships from the `omnia` repo would create a dependency cycle (`omnia → backends → omnia`).

Consequences that shape the whole design:

- **omnia** ships only the *mechanism* plus its in-memory `*Default` backends.
- The configurable binaries live **downstream** — in the `backends` repo (or a dedicated distribution crate), where the concrete backends already are. There is deliberately more than one of them (§6.2): a single universal binary would pay the union of every backend's dependency, CVE, and build cost for every deployment.
- Selecting an implementation a binary was never compiled with is impossible without a rebuild — *unless* the backend runs out-of-process (§7.1). In-process dynamic loading (native plugins, wasm-component backends) is rejected outright (§7.2). Both are out of scope for v1.

## 4. Proposed design — the dynamic seam

### 4.1 Boxed context fields

A *dynamic* backend bundle carries one `Arc<dyn WasiXxxCtx>` per enabled interface instead of a concrete backend type, and plugs into the library `omnia::StoreCtx<B>` unchanged:

```rust
#[derive(Clone)]
pub struct DynamicBackends {
    omnia_wasi_keyvalue: Arc<dyn WasiKeyValueCtx>,
    omnia_wasi_sql:      Arc<dyn WasiSqlCtx>,
    // …one per enabled interface
}
```

The per-interface blanket `WasiXxxView for StoreCtx<B>` changes to borrow the trait object **shared** (`&dyn WasiXxxCtx`) rather than `&mut`. This is sound because the context traits are `&self` today (`WasiKeyValueCtx::open_bucket(&self, …)`, `WasiOtelCtx::export_*(&self, …)`, …). `store()` then clones an `Arc` (cheap) instead of cloning a concrete backend. Interfaces whose context methods genuinely need `&mut` are called out in §10.

### 4.2 The backend registry

omnia gains a `Backends` registry: for each interface, a map from an implementation **name** to a constructor returning the boxed context:

```rust
// constructor signature, per interface
type KeyValueCtor =
    Box<dyn Fn() -> FutureResult<Arc<dyn WasiKeyValueCtx>> + Send + Sync>;
```

The downstream binary registers the concrete backends it compiled in:

```rust
let mut backends = Backends::new();
backends.keyvalue("memory", || async { Ok(Arc::new(KeyValueDefault::connect().await?) as _) });
backends.keyvalue("redis",  || async { Ok(Arc::new(Redis::connect().await?) as _) });
backends.sql("postgres",    || async { Ok(Arc::new(Postgres::connect().await?) as _) });
```

At startup, `Runtime::new` reads the manifest's backend selections, resolves each interface against the registry, and **fails fast** if a requested name was not compiled in (listing the names that *are* available).

### 4.3 The macro surface

A dynamic mode of `runtime!` (or a sibling `dynamic_runtime!`) takes *interfaces only* — no backend type — and generates the boxed bundle, the `Backends` registry type with its `register_*` methods, and a `run` that threads the populated registry through:

```rust
omnia::runtime!({
    mode: dynamic,
    hosts: { WasiHttp, WasiKeyValue, WasiSql, WasiOtel },
});

// the binary author supplies the compiled-in implementations
fn backends() -> Backends { /* register_* calls as in §4.2 */ }
```

### 4.4 Manifest `[backends]`

A new optional table in `crates/omnia/src/deployment/manifest.rs` maps each interface string to an implementation name. Omitting it (or the whole file) keeps today's zero-config behaviour: every interface falls back to its in-memory `*Default`.

```toml
[backends]
keyvalue = "redis"      # default: "memory"
sql      = "postgres"
# http, otel unspecified -> their defaults
```

## 5. Host (capability / trigger) selection

Selecting *which hosts* are active is the easy half:

- All listed hosts are linked into the one shared `Linker`. Capability hosts (`WasiKeyValue`, `WasiBlobstore`, `WasiOtel`, `WasiSql`, …) are inert when a guest does not import them, so they can always be linked.
- Which **trigger servers** actually run (`WasiHttp`, `WasiMessaging`, `WasiWebSocket`) becomes config-driven: the `servers` vector the generated `run` builds is populated only for enabled triggers.
- The command (`mode: command`) vs long-lived co-listing rule, today fixed by the compile-time `mode` argument of `runtime!` (driven by `omnia::Mode` in `crates/omnia/src/runtime.rs`), moves to a startup check in dynamic mode — or a command deployment stays a separate binary. See §10.

## 6. Distribution: what actually ships

### 6.1 Where this lives

- **omnia** — the mechanism: the dynamic macro mode, the `Backends` registry, the `[backends]` manifest section, and the `Arc<dyn _>` view change. Plus its in-memory `*Default` impls, which need no external services.
- **backends** — the configurable binaries (§6.2) that register the real implementations and call `run`. It already depends on omnia, so there is no cycle.
- A separate distribution repo is warranted only if a deployment must combine backends from multiple source repos.

A binary built purely from omnia's `*Default` backends is possible but uninteresting — there is exactly one default per interface, so there is nothing to select. The feature only earns its keep downstream, where multiple real backends exist for an interface.

### 6.2 Curated profile binaries, not one universal binary

Nothing in §4 requires exactly one configurable binary — and there should not be one. A binary compiled with *every* backend charges every deployment the union of all their costs:

- **CVE surface is the union.** A vulnerability in the Kafka client forces a rebuild and re-ship for deployments that never select Kafka.
- **One lockfile for everything.** Every backend SDK must coexist in a single dependency graph, permanently.
- **The build inherits the fussiest backend.** `omnia-kafka`'s `rdkafka` (cmake build, vendored OpenSSL) is the lone native-build dependency in the `backends` workspace; everything else (`redis`/rustls, `deadpool-postgres`, `async-nats`, `mongodb`, `azure_*`, `opentelemetry-proto`) is pure Rust and cross-compiles freely.
- **The tested surface becomes combinatorial** instead of "the binary": CI must exercise selectable combinations, not one artifact.

Instead, `backends` CI ships a small family of **profile binaries**, each a coherent bundle with §4 selection *inside* it — for example:

- `omniad` — the base/protocol profile: Redis, Postgres, NATS, plus the in-memory defaults.
- `omniad-azure` — Blob, Table, Vault, Identity.
- `omniad-kafka` — quarantines the `rdkafka` native build.

(Exact profile boundaries are an open question, §10.) Profiles are published to an artifact registry — OCI images or release binaries — where signing, digest pinning, and mirroring are already solved. The operator pulls an artifact by name instead of compiling; "dynamic download" happens at the **artifact** level, which container infrastructure does well, not at the code-loading level, where the ABI and trust problems live (§7.2). Recompiles remain a central CI concern, never an operator one, and the `omnia.toml` schema is identical across profiles.

### 6.3 Register protocols, not vendors

Most of the apparent backend matrix collapses onto a handful of wire protocols; vendor variety lives at the endpoint URL, which `FromEnv` already resolves at runtime:

- **RESP** covers Redis, Valkey, ElastiCache, MemoryDB, Dragonfly, Azure Cache.
- **Postgres wire** covers Postgres, CockroachDB, Neon, AlloyDB, Timescale.
- **S3 API** covers essentially every blob store, including most clouds' compatibility endpoints.
- **OTLP** covers every observability vendor.
- **Kafka protocol** covers Kafka, Redpanda, WarpStream.

Registry names should therefore identify a protocol or implementation (`"redis"`, `"postgres"`, `"s3"`), never a vendor: one compiled-in client per protocol covers a wide deployment matrix and keeps the base profile small and pure Rust. Genuinely proprietary interfaces (Azure Table's OData surface, Key Vault) belong in vendor profiles.

## 7. Future: an open backend set without recompiling

### 7.1 Out-of-process over wRPC — the escape hatch

The recompile constraint of §3 dissolves only when a backend runs **out of process**, reached over the host-mediated wRPC transport (see [wrpc-cluster](wrpc-cluster.md)). Such a backend is just another `WasiXxxCtx` implementation — a forwarding context that relays each call to a remote endpoint — registered and selected by the same `[backends]` mechanism (e.g. `keyvalue = "wrpc"`, endpoint from env). This lets a fixed binary use backends it was never compiled against.

The per-call marshaling tax is proportionate for the backends this targets:

- They are **network-bound already**: a keyvalue `get` against Redis pays a service round-trip of ~100µs–1ms; a colocated Unix-socket hop to a sidecar adds tens of microseconds.
- The guest→host crossing **already serializes** through the canonical ABI; the forwarding context re-encodes data that was just lifted — an incremental cost, not a new category of cost.
- Where the tax *would* hurt — the in-memory `*Default`s and any hot local-state path — is exactly where it must not be used: those stay in-process.

Adoption is per interface and opt-in, only where a deployment needs a backend outside its profile. It is a natural layer on the §4 seam but is **not** part of v1.

### 7.2 Rejected: in-process dynamic loading

Two "load code into the running binary" alternatives avoid both compilation-in and (for the first) serialization, and both are rejected:

- **Native plugins** (`dlopen` + `abi_stable`/`stabby`-style stable-ABI shims). The `WasiXxxCtx` traits are async and traffic in `Arc<dyn Bucket>` and `anyhow::Result` — none of which crosses an FFI boundary without a callback-based redesign. Add rustc-version coupling, Tokio-across-dylib hazards, and the trust problem of downloading native code that receives live credentials, with fewer guardrails than OCI artifacts.
- **Backends as wasm components** (e.g. pulled from a wkg registry). Backends exist to hold precisely what guests cannot: native TLS, pooled connections, epoll-driven I/O, librdkafka. The driver SDKs largely do not compile to `wasm32-wasip2` (rdkafka is C and never will); components share no memory, so connection pooling breaks; every `WasiXxxCtx` would need a WIT re-expression; and every call pays a second component boundary. The out-of-process form (§7.1) provides the same openness without re-implementing drivers.

## 8. Acceptance criteria

1. One prebuilt binary runs two deployments that differ *only* in `omnia.toml [backends]` (e.g. `keyvalue = "memory"` vs `"redis"`) with no recompile.
2. Requesting an implementation not compiled into the binary fails fast at startup with a clear error naming the available implementations.
3. Zero-config (no `[backends]`, or no manifest at all) still runs on the in-memory `*Default` backends.
4. Backend connection stays env-driven via `FromEnv`; no new config surface for connection details.
5. The compile-time `runtime!` path is **unchanged** for existing users and examples.
6. `make lint` and `cargo make ci` stay green.
7. Switching profiles (e.g. `omniad` → `omniad-kafka`) is an artifact pull, not a rebuild; the `omnia.toml` schema is identical across profiles.

## 9. Risks and invariants

- **Backends stay behind the `WasiXxxCtx` boundary.** Selection is by an abstract implementation name; the choice is never visible to guests.
- **No dependency cycle.** omnia never depends on a concrete backend; resolution is by name through a registry the downstream binary populates.
- **The zero-cost path is preserved.** Static `runtime!` users pay nothing. The dynamic path adds one `Arc` + vtable indirection per host call — which the existing `&mut dyn` view already incurs.
- **Fail fast, never silently degrade.** A missing requested backend is an error, not a silent fallback. The only implicit choice is the documented zero-config default.
- **No universal binary.** The union dependency/CVE/build cost is avoided by curated profiles (§6.2); "configure, don't compile" holds *within* a profile, and moving between profiles is an artifact pull.
- **Backends are not perfectly interchangeable.** Atomicity, transaction semantics, and consistency differ per implementation, and a config swap is easier to make than a reviewed code change. Deployments must be tested against the backend they select; the profile split keeps that matrix small.

## 10. Open questions

1. `Arc<dyn _>` + shared (`&`) borrow vs `Box<dyn _>` + a `clone_box` for any interface whose context methods require `&mut`. Audit each `WasiXxxCtx`; the ones reviewed (`keyvalue`, `otel`) are `&self`.
2. Generated `main` ergonomics in dynamic mode: how the generated `main` receives the populated `Backends` — a registration-fn path, a builder closure passed to `run`, or `inventory`-style auto-registration.
3. Registry shape: macro-generated per-interface typed maps (explicit, type-safe) vs a single type-keyed heterogeneous map.
4. The command vs long-lived trigger guard as a startup check, vs keeping a command deployment a separate binary.
5. Fold this into `runtime!` as a `mode`, or ship a separate `dynamic_runtime!` macro to keep the static path's parsing simple.
6. Profile boundaries and publication: which backends ship in which `omniad-*` profile, and where profiles are published (OCI registry vs release binaries).
7. A pure-Rust Kafka client (`rskafka`) instead of `rdkafka` would drop the only native build and let Kafka join the base profile — its feature coverage (consumer groups, transactions, SASL variants) needs auditing.

## 11. References

- `crates/host-macros/src/runtime/{codegen,parse}.rs` — the macro this design extends.
- `crates/omnia/src/host.rs` — `Backend`, `FromEnv`, `Host`, `Server`.
- `crates/omnia/src/deployment/manifest.rs` — the `omnia.toml` schema gaining `[backends]`.
- `crates/wasi-keyvalue/src/host.rs` — a representative `WasiXxxCtx` trait + view, i.e. the dynamic seam this design leans on.
- [rfc-58-backend-router](rfc-58-backend-router.md) — per-call *model* backend routing behind `wasi-model`; an orthogonal selection mechanism.
- [wrpc-cluster](wrpc-cluster.md) — the host-mediated transport the §7 extension rides.
