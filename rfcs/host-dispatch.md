# Design: Centralising Host→Guest Calls — the `HostDispatch` Seam

> Status: Design proposal — a refactor of landed code, no behaviour change.
>
> The host→guest dispatch primitive (`dispatch`) and its type-erased
> seam (`HostDispatch`) have landed in `crates/omnia/src/dispatch.rs`, and
> `wasi-model`'s `resolve` rides them. This document proposes realigning the seam
> so the floor stays generic (Law 2) and the `resolve` semantics live in
> `wasi-model`, where they belong.

## 1. The problem

`crates/omnia/src/dispatch.rs` welds two concerns together:

- a **generic** host→guest primitive — `dispatch` (instantiate a known
  guest fresh, call a known `interface/func` with `Vec<Val>`, return `Vec<Val>`;
  depth-bounded, resource-rejecting, instance-per-call); and
- **`wasi-model`'s `resolve` semantics**, sitting in the floor: the seam trait
  method `HostDispatch::resolve`, the `RESOLVE_FUNC = "resolve"` convention,
  `resolve_interface` (find the interface exporting `resolve`), and
  `vals_to_bytes` (the `references`-shelf return shape).

The seam the `runtime!` macro threads into *every* store is shaped around one
consumer. That contradicts the file's own stated Law 2 — "the floor stays
generic … it never parses any consumer scheme" — and means a second host wanting
a dynamic host→guest call would have to add a verb to a floor trait.

The **mechanism is sound**: type-erasing the runtime behind `Arc<dyn …>` and
threading it through `StoreCtx` is the right way to break the
`StoreCtx ⇄ Runtime` cycle (it mirrors the per-store `wrpc` state). Only the
*trait surface* and *helper placement* are wrong.

## 2. The realignment

Make `HostDispatch` a thin, generic, type-erased mirror of `dispatch`,
and move the `resolve` semantics into `wasi-model`.

**Floor (`omnia`)** — one generic seam, no `wasi-model` vocabulary:

```rust
pub trait HostDispatch: Send + Sync + 'static {
    /// Invoke `target`'s `interface`/`func` with `args` (instance-per-call,
    /// depth-bounded, resources rejected). `None` discovers the unique exported
    /// interface carrying a function named `func`.
    fn invoke(
        &self, target: GuestId, interface: Option<String>, func: String, args: Vec<Val>,
    ) -> FutureResult<Vec<Val>>;
}
```

The blanket `impl<R: Runtime> HostDispatch for R` resolves the interface (when
`None`) and delegates to `dispatch`. `RESOLVE_FUNC` and `vals_to_bytes`
leave the floor entirely.

**`wasi-model`** — owns the `resolve` verb, the discover-by-name choice, and the
bytes shape, composing the floor seam:

```rust
const RESOLVE_FUNC: &str = "resolve";

// inside ToolHost::resolve
let results = dispatch
    .invoke(GuestId::from(target), None, RESOLVE_FUNC.to_owned(),
            vec![Val::String(reference.name)])
    .await?;
vals_to_bytes(results) // moved out of the floor
```

`wasi-model` already depends on `omnia` and `wasmtime` (so `Val` / `GuestId` are
in hand) and `omnia` does **not** depend on `wasi-model`, so the move is
dependency-correct.

## 3. Where interface discovery lives

The `references` interface name is deliberately *not* fixed — the floor finds it
by the exported function name (`examples/model/wit/world.wit`: "invoked by
convention … without naming this interface"). That discovery needs the
registry/engine, so it must stay in the floor — but "find the interface
exporting function X" is a structural component-model query that names no
consumer scheme, so it stays Law-2-clean. It is today's `resolve_interface`
generalised to take `func: &str` instead of the hard-coded `RESOLVE_FUNC`.

## 4. Why this is the central host→guest story

- The floor keeps **one** generic host→guest entry point (`dispatch`)
  with **one** type-erased face (`HostDispatch::invoke`). The depth bound is
  already shared — host→guest and guest→guest both pass through
  `DispatchHandle::enter` — so this finishes that alignment.
- The macro-threaded `host_dispatch` field stops being a `wasi-model` leak and
  becomes a generic floor capability, exactly parallel to the `wrpc` field
  beside it. Any future host gets dynamic host→guest calls for free.
- Triggers (`wasi-http`, `wasi-messaging`) stay out of scope: they use typed,
  compile-time bindings (`ServiceIndices` / `run_concurrent`), a different and
  appropriate path. This seam is for *dynamic, runtime-resolved* calls only.

## 5. Blast radius

- **`backends` repo: no change.** genai / cursor only touch
  `omnia_wasi_model::ToolHost`; `ToolHost::resolve`'s signature is unchanged.
- **`omnia`:** retype one trait + its blanket impl, generalise one helper, delete
  two `wasi-model`-specific items (`RESOLVE_FUNC`, `vals_to_bytes`).
- **`wasi-model`:** gain a small `resolve` / `vals_to_bytes` helper; the
  `WasiModelCtxView.host_dispatch` field type is unchanged.
- **macro / tests:** keep threading `Arc<dyn HostDispatch>` exactly as now; only
  the trait's method set changes.

## 6. References

- [wrpc-cluster.md](wrpc-cluster.md) — guest→guest host-mediated dynamic linking;
  shares the dispatch-depth bound and the instance-per-call guarantee.
- [wasi-model.md](wasi-model.md) — the `resolve` consumer (the `ToolHost`
  callback into a `references` shelf).
- `crates/omnia/src/dispatch.rs` — `dispatch`, `HostDispatch`, and the
  `resolve` helpers being realigned.
