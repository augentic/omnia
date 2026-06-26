# Design: Runtime Interface Extraction

> Status: Design proposal — refactors the `runtime!` macro expansion into a documented, testable `omnia` API. No behaviour change; improves ergonomics for macro-generated and hand-written runtimes alike.
>
> Owns: `StoreBase`, `#[derive(StoreContext)]`, `omnia::serve`, defaulted `Runtime` methods, and the corresponding shrink of `crates/runtime-macro`. Depends: the landed `Runtime` trait, `RegistryBuilder` / `Compiled`, and the host-crate `omnia_wasi_view!` pattern.

## 1. Abstract

The `runtime!` proc-macro (`crates/runtime-macro/src/expand.rs`) currently emits ~180 lines of fixed boilerplate for every deployment: a `Context` struct, lifecycle orchestration (`start`), per-store construction policy, three foreign trait impls (`WasiView`, `WrpcView`, `HasLimits`), and one `omnia_wasi_view!` invocation per enabled host. Only a small fraction of that output is genuinely deployment-specific — the set of `(host, backend)` pairs and the optional `main` toggle.

This design extracts the fixed parts into the `omnia` crate as real, documented, testable code. The macro becomes a thin wiring layer. Hand-written runtimes (e.g. `crates/wasi-model/tests/replay.rs`) reuse the same surface instead of re-implementing identical trait impls.

## 2. Problem

### 2.1 What the macro generates today

For a typical deployment (`examples/http/runtime.rs`):

```rust
omnia::runtime!({
    main: true,
    hosts: {
        WasiHttp: HttpDefault,
        WasiOtel: OtelDefault,
    }
});
```

`expand.rs` emits a `mod runtime` containing:

| Generated item | Fixed (same every deployment) | Variable (deployment-specific) |
|---|---|---|
| `run()` | compile → new → start skeleton | `StoreCtx` type, `Context::new` body |
| `Context` struct | `registry` field | one field per backend |
| `Context::new` | `registry: Arc::new(compiled.build()?)` | host `.host::<H>()` calls, backend connects |
| `Context::start` | **entire orchestration** | only the server list |
| `impl Runtime` | `options()`; WASI/limits part of `store()` | `registry()`, variable store fields |
| `StoreCtx` | 5 base fields | per-host backend fields |
| `WasiView` / `HasLimits` / `WrpcView` | **all three, verbatim** | — |
| `omnia_wasi_view!` × N | pattern is uniform | host path + field name |
| `main` | CLI dispatch | `gen_main` toggle |

The fixed portion is duplicated in every generated runtime and reproduced by hand in integration tests (`replay.rs` lines 49–127).

### 2.2 Why this matters

- **Discoverability.** The runtime lifecycle (`drive_epoch` → `sample_pool` → `serve_links` → `try_join_all(servers)`) is buried in generated tokens, not in `omnia`'s public API or docs.
- **Testability.** `StoreCtx` construction policy (WASI inheritance, memory limits, `host_dispatch` threading) cannot be unit-tested without invoking the macro.
- **Hand-written runtimes.** Any test or custom deployment must copy the same ~80 lines of trait impls. The `Runtime` trait already defaults `build_store` and `instantiate`; the rest should follow the same precedent.
- **Host-crate coupling.** Every `omnia_wasi_view!` macro in the 12 WASI host crates reaches for flat field names (`self.table`, and in `wasi-model`, `self.host_dispatch`). That implicit contract is undocumented and enforced only by convention.

## 3. Goals

1. **Single source of truth** for store-context base fields, their construction policy, and the three fixed trait impls.
2. **Shrink `runtime!`** to emit only deployment-specific wiring: backend fields + connects, host links, per-host store fields, server list, optional `main`.
3. **Same ergonomic surface** for macro-generated and hand-written runtimes.
4. **No behaviour change** — a pure refactor; `cargo make ci` stays green.

## 4. Non-goals

- Changing the `runtime!` input syntax (`hosts: { WasiHttp: HttpDefault, … }`).
- A `#[derive(Runtime)]` on `Context` in the first pass (optional follow-on; see §8).
- Replacing per-host-crate `omnia_wasi_view!` macros with generated code inside each host crate (they stay; only their field paths change).
- Changing `RegistryBuilder`, `Compiled`, or the guest registry / dispatch machinery.

## 5. Design

### 5.1 `StoreBase` — fixed per-store state

New type in `omnia` (`crates/omnia/src/store.rs` or similar):

```rust
pub struct StoreBase {
    pub table: ResourceTable,
    pub wasi: WasiCtx,
    pub limits: StoreLimits,
    pub wrpc: WrpcState,
    pub host_dispatch: Arc<dyn HostDispatch>,
}

impl StoreBase {
    /// Absorbs the WASI-inheritance + memory-limit + wRPC policy currently
    /// inlined in the generated `Runtime::store()` (expand.rs lines 144–164).
    pub fn new(options: &RuntimeOptions, host_dispatch: Arc<dyn HostDispatch>) -> Self {
        StoreBase {
            table: ResourceTable::new(),
            wasi: WasiCtxBuilder::new()
                .inherit_env()
                .inherit_stdin()
                .stdout(tokio::io::stdout())
                .stderr(tokio::io::stderr())
                .build(),
            limits: StoreLimitsBuilder::new()
                .memory_size(options.max_memory_bytes)
                .build(),
            wrpc: WrpcState::new(),
            host_dispatch,
        }
    }
}
```

Replaces the five fixed fields currently emitted on `StoreCtx`:

```rust
pub table: ResourceTable,
pub wasi: WasiCtx,
pub limits: StoreLimits,
pub wrpc: omnia::WrpcState,
pub host_dispatch: Arc<dyn omnia::HostDispatch>,
```

### 5.2 `#[derive(omnia::StoreContext)]` — fixed trait impls + host views

New derive macro (in `omnia-runtime-macro` or a sibling crate). The generated `StoreCtx` becomes:

```rust
#[derive(omnia::StoreContext)]
pub struct StoreCtx {
    #[base]
    base: StoreBase,
    #[wasi(omnia_wasi_http)]
    omnia_wasi_http: HttpDefault,
    #[wasi(omnia_wasi_otel)]
    omnia_wasi_otel: OtelDefault,
}
```

**Attributes:**

| Attribute | Meaning |
|---|---|
| `#[base]` | Marks the `StoreBase` field. Exactly one required. |
| `#[wasi(path::to::host_crate)]` | Emits `path::omnia_wasi_view!(StoreCtx, field_ident)` for this backend field. |

**Generated impls** (against the `#[base]` field):

```rust
impl WasiView for StoreCtx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.base.wasi,
            table: &mut self.base.table,
        }
    }
}

impl WrpcView for StoreCtx {
    type Invoke = LinkClient;
    fn wrpc(&mut self) -> WrpcCtxView<'_, LinkClient> {
        self.base.wrpc.view(&mut self.base.table)
    }
}

impl HasLimits for StoreCtx {
    fn limits(&mut self) -> &mut StoreLimits {
        &mut self.base.limits
    }
}
```

Plus one `omnia_wasi_view!` invocation per `#[wasi(...)]` field.

This collapses expand.rs lines 183–210 (three impls + N macro calls) into a single derive co-located with the struct definition.

### 5.3 `omnia::serve` — fixed lifecycle orchestration

New function in `omnia` (`crates/omnia/src/runtime.rs`, alongside `drive_epoch` / `sample_pool`):

```rust
pub async fn serve<R: Runtime>(
    runtime: &R,
    servers: Vec<BoxFuture<'_, Result<()>>>,
) -> Result<()>
where
    R::StoreCtx: WasiView + WrpcView + 'static,
{
    drive_epoch(
        runtime.registry().engine().clone(),
        runtime.options().epoch_tick,
    );
    sample_pool(
        runtime.registry().engine().clone(),
        runtime.options().pool_metrics_interval,
    );
    serve_links(runtime)
        .await
        .context("wiring host-mediated link serve side")?;
    try_join_all(servers).await?;
    Ok(())
}
```

Absorbs the entire `Context::start()` body (expand.rs lines 98–129). The only variable input is the server list; `Context::start` is deleted and `run()` calls `serve` directly.

Note: `serve_links` requires `R::StoreCtx: WasiView + WrpcView + 'static` (`dispatch.rs` lines 511–515); the where-clause on `serve` mirrors that.

### 5.4 Defaulted `Runtime::options()`

Both the generated `Runtime` impl and the hand-written `TestRuntime` in `replay.rs` implement `options()` identically: `self.registry().options()`.

Add a default method on the `Runtime` trait (alongside the existing defaults for `build_store` and `instantiate` in `traits.rs`):

```rust
fn options(&self) -> &RuntimeOptions {
    self.registry().options()
}
```

Removes one generated method and one duplicate in every hand-written runtime.

### 5.5 Host-crate `omnia_wasi_view!` update

The derive requires host view macros to reference `self.base.table` (and `self.base.host_dispatch` where applicable) instead of flat `self.table` / `self.host_dispatch`.

**Affected crates** (~11 of 12):

| Crate | Change |
|---|---|
| `wasi-http` | `self.table` → `self.base.table` in `.as_view(&mut …)` |
| `wasi-otel`, `wasi-vault`, `wasi-sql`, `wasi-messaging`, `wasi-keyvalue`, `wasi-jsondb`, `wasi-identity`, `wasi-blobstore`, `wasi-websocket` | `table: &mut self.table` → `table: &mut self.base.table` |
| `wasi-model` | both `self.table` and `self.host_dispatch` → `self.base.*` |
| `wasi-config` | **unchanged** — reads only `self.$field_name`, never `table` |

The pattern is mechanical and uniform across host crates.

## 6. Constraints

Two Rust language constraints dictate the shape above; neither is an arbitrary choice.

### 6.1 Borrow checker — `base` must be a named field

A trait accessor (`trait HasStoreCore { fn store_core(&mut self) -> &mut StoreBase }`) fails for host views: `WasiHttpView::http` needs `&mut self.<backend>` and `&mut self.<table>` simultaneously. Calling `store_core(&mut self)` borrows all of `self`, conflicting with the backend borrow. Disjoint-field borrows only work through a concrete place expression — `self.base.table` alongside `self.omnia_wasi_http`. The base must therefore be a struct field with a name the derive knows (via `#[base]`).

### 6.2 Orphan rule — derive, not blanket impls

`HasLimits` is `omnia`-owned and *could* be blanket-impl'd, but `WasiView` (`wasmtime_wasi`) and `WrpcView` (`wrpc_wasmtime`) are foreign traits — `impl<T: LocalMarker> ForeignTrait for T` is illegal. They must be implemented on the concrete `StoreCtx`, which is exactly what a derive does. Folding all three impls into one derive keeps the surface consistent.

## 7. Before → after

### 7.1 Generated `mod runtime` (after)

```rust
mod runtime {
    use omnia::{serve, Backend, Compiled, Registry, Runtime, StoreBase, StoreContext, …};

    #[derive(StoreContext)]
    pub struct StoreCtx {
        #[base] base: StoreBase,
        #[wasi(omnia_wasi_http)] omnia_wasi_http: HttpDefault,
        #[wasi(omnia_wasi_otel)] omnia_wasi_otel: OtelDefault,
    }

    #[derive(Clone)]
    struct Context {
        registry: Arc<Registry<StoreCtx>>,
        http_default: HttpDefault,
        otel_default: OtelDefault,
    }

    impl Context {
        async fn new(mut compiled: Compiled<StoreCtx>) -> Result<Self> {
            compiled.host::<WasiHttp>()?;
            compiled.host::<WasiOtel>()?;
            let (http_default, otel_default) = tokio::try_join!(
                <HttpDefault as Backend>::connect(),
                <OtelDefault as Backend>::connect(),
            )?;
            Ok(Self {
                registry: Arc::new(compiled.build()?),
                http_default,
                otel_default,
            })
        }
    }

    impl Runtime for Context {
        type StoreCtx = StoreCtx;
        fn registry(&self) -> &Registry<StoreCtx> { &self.registry }
        fn store(&self) -> StoreCtx {
            StoreCtx {
                base: StoreBase::new(self.options(), Arc::new(self.clone())),
                omnia_wasi_http: self.http_default.clone(),
                omnia_wasi_otel: self.otel_default.clone(),
            }
        }
        // options(), build_store(), instantiate() — all defaulted
    }

    pub async fn run(wasm: Option<PathBuf>, config: Option<PathBuf>) -> Result<()> {
        let compiled = RegistryBuilder::new()
            .wasm(wasm).config(config)
            .compile::<StoreCtx>().await?;
        let runtime = Context::new(compiled).await?;
        serve(&runtime, vec![
            Box::pin(WasiHttp.run(&runtime)),
            Box::pin(WasiOtel.run(&runtime)),
        ]).await
    }
}
```

Everything remaining is genuinely deployment-specific.

### 7.2 Hand-written runtime (`replay.rs`, after)

```rust
#[derive(StoreContext)]
struct TestCtx {
    #[base] base: StoreBase,
    #[wasi(omnia_wasi_model)] model: Box<dyn WasiModelCtx>,
}

#[derive(Clone)]
struct TestRuntime {
    registry: Arc<Registry<TestCtx>>,
    backend: BackendFactory,
    stores_built: Arc<AtomicUsize>,
}

impl Runtime for TestRuntime {
    type StoreCtx = TestCtx;
    fn registry(&self) -> &Registry<TestCtx> { &self.registry }
    fn store(&self) -> TestCtx {
        self.stores_built.fetch_add(1, Ordering::SeqCst);
        TestCtx {
            base: StoreBase::new(self.options(), Arc::new(self.clone())),
            model: (self.backend)(),
        }
    }
}
```

The three manual trait impls (`WasiView`, `HasLimits`, `WrpcView`, `WasiModelView`) collapse to the derive + one host macro call.

### 7.3 `Runtime` trait surface (after)

Required methods shrink to two:

```rust
pub trait Runtime: Clone + Send + Sync + 'static {
    type StoreCtx: Send + HasLimits;
    fn store(&self) -> Self::StoreCtx;
    fn registry(&self) -> &Registry<Self::StoreCtx>;

    // defaulted:
    fn options(&self) -> &RuntimeOptions { … }
    fn build_store(&self, data: Self::StoreCtx) -> Store<Self::StoreCtx> { … }
    fn instantiate(…) -> impl Future<Output = Result<Instance>> + Send { … }
}
```

## 8. Optional follow-on: `#[derive(Runtime)]`

If hand-written runtimes become common, a second derive on `Context` could generate `registry()` / `store()` from attributes:

```rust
#[derive(Clone, Runtime)]
#[runtime(store = StoreCtx)]
struct Context {
    #[runtime(registry)] registry: Arc<Registry<StoreCtx>>,
    #[runtime(store = omnia_wasi_http)] http_default: HttpDefault,
}
```

Defer until the `StoreContext` derive + `StoreBase` + `serve` land. The attribute coupling between `Context` field names and `StoreCtx` field names adds proc-macro machinery that only earns its place if manual runtimes are frequent.

## 9. Migration plan

1. **Land `StoreBase`** in `omnia` with `new()` and re-export from crate root.
2. **Land `serve()`** and default `options()` on `Runtime`.
3. **Implement `#[derive(StoreContext)]`** in `omnia-runtime-macro` (or sibling).
4. **Update all host-crate `omnia_wasi_view!` macros** (`self.table` → `self.base.table`; `wasi-model` also `host_dispatch`).
5. **Rewrite `expand.rs`** to emit the slim shape (§7.1).
6. **Rewrite `replay.rs` `TestCtx`** to use the derive (§7.2) — becomes a consumer-facing test of `StoreContext`.
7. **Verify** all examples and `cargo make ci` stay green.

Steps 1–2 are independent and can land first with no disruption. Steps 3–4 must land together (derive emits `self.base.*`; host macros must understand it). Step 5 depends on 3–4.

## 10. Open questions

- **Derive crate location.** Co-locate `StoreContext` with `runtime!` in `omnia-runtime-macro`, or split into `omnia-store-macro`? Co-location keeps one proc-macro crate; splitting keeps `runtime!` focused on wiring.
- **`StoreCtx` visibility.** Today the macro emits `pub struct StoreCtx`. Should the derive preserve that, or should visibility be an attribute (`#[store_context(vis = "pub")]`)? Default `pub` matches current behaviour.
- **Field naming.** The macro currently snake-cases backend type names (`HttpDefault` → `http_default`) for both `Context` and `StoreCtx` fields. The derive should accept explicit field names (the struct author chooses them); the macro continues generating those names. No change to naming policy.
- **`wasi-config` long-term.** Its view impl ignores `table` entirely. If more hosts follow that shape, consider a `#[wasi(..., no_table)]` attribute or a separate derive path. Not needed for the initial migration.

## 11. Acceptance criteria

1. `StoreBase`, `serve`, and `#[derive(StoreContext)]` are public, documented `omnia` API.
2. `runtime!` emits only deployment-specific wiring (§7.1); `expand.rs` shrinks by roughly half.
3. `replay.rs` uses `StoreBase` + `#[derive(StoreContext)]`; its manual `WasiView` / `HasLimits` / `WrpcView` impls are deleted.
4. All 12 host crates' `omnia_wasi_view!` macros compile against `self.base.table` (and `self.base.host_dispatch` where applicable).
5. Every existing example (`examples/http`, `examples/linking`, `examples/model`, …) builds and runs unchanged from the caller's perspective.
6. `cargo make ci` stays green.
7. No behaviour change: epoch driving, pool metrics, link serve wiring, and per-store WASI policy are identical before and after.

## 12. Risks and invariants

- **The flat-field contract moves to `base`.** Any code outside host view macros that reaches for `store.table` or `store.host_dispatch` directly must update. Grep for usages before landing step 4.
- **Derive + host macro must land atomically.** A partial migration (derive emitting `self.base.*` before host macros are updated, or vice versa) breaks every deployment.
- **Law 2 preserved.** `StoreBase` and `serve` carry no consumer vocabulary; `host_dispatch` remains the generic host→guest seam.
- **Instance-per-call unchanged.** `StoreBase::new` runs once per `store()` call; `build_store` / `instantiate` defaults are untouched.
