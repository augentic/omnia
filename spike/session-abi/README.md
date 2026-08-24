# Session ABI spike — verdict

Phase 1 of the `omnia:model` 0.2 session design: throwaway code proving
`stream<tool-call>` and `future<result<reply, error>>` nested in a record
returned from an async host import, through wasmtime 47 host bindgen and
wit-bindgen 0.60 guest bindings, plus the end-drop matrix.

**Verdict: GO. The record-nested shape works end to end as sketched in the
canvas. Neither fallback (flattened tuple, two-call handshake) was needed.**

Verified with rustc 1.98.0 (stable), wasmtime 47.0.4 (pinned `47.0.3` +
`component-model-async` feature), wit-bindgen 0.60.0 (`async-spawn`),
`wasm32-wasip2` guest target. All 7 matrix tests pass in ~0.2 s.

## Build and run

```bash
cd spike/session-abi
cargo build -p spike-guest --target wasm32-wasip2   # guest first
cargo test -p spike-host                            # then the matrix
```

## What was proven

The WIT under test is [wit/spike.wit](wit/spike.wit):

```wit
record session {
    calls: stream<tool-call>,
    reply: future<result<reply, error>>,
}
create: async func(request: string, results: stream<tool-result>) -> result<session, error>;
```

- **Host bindgen** (`imports: { default: store | trappable }`) generates
  `Session { calls: StreamReader<ToolCall>, reply: FutureReader<Result<Reply, Error>> }`
  with full `Lift`/`Lower` impls, and the host trait
  `HostWithStore::create(accessor: &Accessor<Ctx, Self>, request: String,
  results: StreamReader<ToolResult>) -> wasmtime::Result<Result<Session, Error>>`.
- **Guest bindgen** generates the mirror `Session` with
  `wit_bindgen::rt::async_support::{StreamReader, FutureReader}` fields, plus
  top-level `wit_stream::new()` / `wit_future::new()` constructors.
- Runtime lifting/lowering of the record round-trips both directions:
  the guest-created results reader crosses as a param, and the host-created
  calls reader + reply future cross back inside the returned record.
- The canvas's guest sugar shape works verbatim: `futures::join!` over the
  calls-read loop and the reply future inside one async-lifted export
  (`FutureReader` is `IntoFuture`, not `Future` — `join!` needs
  `.into_future()`).

## End-drop matrix results ([host/tests/matrix.rs](host/tests/matrix.rs))

Every case is timeout-wrapped; all terminate promptly. Guest events are
asserted from `run`'s return value, host events from the backend's channels.

| Case | Observed behavior |
|---|---|
| `round_trip` | 3 calls, 3 results, reply resolves after host closes calls. |
| `parallel_calls` | 3 calls in flight; guest answers in reverse; host correlates by id — results stream is safely unordered. |
| `guest_drops_session` | Guest drops calls reader + reply future mid-loop; host's mpsc/oneshot ends observe closure (`Sender::closed()` fires) **while the instance is still live**. Producer state is dropped promptly, not at store teardown. |
| `host_budget_exhausted` | Host closes calls (producer returns `StreamResult::Dropped`) and resolves reply with `budget-exhausted`; guest loop ends via `next() -> None`, error surfaces from the future. |
| `backend_failure` | Results consumer reports `StreamResult::Dropped`; the guest's pending `write_one` gets the rejected value back (`Some(value)`), then sees calls closed and the failed reply. |
| `guest_closes_results_early` | Guest drops its results writer with a call pending; the host consumer is dropped, `results_rx.recv() -> None` — the host must fail the reply itself (and does). |
| `reply_dropped_without_value` | **Not graceful.** Dropping the host's reply write end without writing errors the future producer, traps the guest, and the error surfaces from `run_concurrent`. The host must *always* write the reply future; budget/deadline failures are WIT `error` values, never writer drops. |

## Host-side API map (for Phase 3)

All in `wasmtime::component` (gated on the `component-model-async` cargo
feature; `Config::concurrency_support` defaults on — no Config changes):

- **Mint a stream toward the guest**: `StreamReader::new(store, impl
  StreamProducer<D, Item = T>)`. `poll_produce` is pull-based; bridge a
  push-style backend with an mpsc channel (`Poll::Ready(Some) ->
  set_buffer + Completed`, `Ready(None) -> Dropped`, `Pending && finish ->
  Cancelled`). `type Buffer = Option<T>` suffices for one-item-per-poll.
- **Mint the reply future**: `FutureReader::new(store, producer)` — the
  blanket `FutureProducer` impl for any `Future<Output = Result<T, E>>`
  (`E: Into<wasmtime::Error>`) makes a mapped tokio oneshot receiver enough.
- **Consume the guest's results stream**: the received `StreamReader<T>`
  is attached via `.pipe(store, impl StreamConsumer<D, Item = T>)`;
  `poll_consume` reads via `Source::read` into a `Vec` and can report
  `StreamResult::Dropped` to model the host abandoning the read end.
- All constructors need store access: call them inside `accessor.with(|access| …)`
  in the concurrent (`store` mode) host trait. The scripted backend itself
  needs **no** store access — it lives entirely behind channels.

## Gotchas recorded along the way

1. **wasmtime 47's `Error` is not `anyhow::Error`.** Use `wasmtime::Error::msg`,
   `wasmtime::ensure!`, and `wasmtime::error::Context`; there is no blanket
   `From<anyhow::Error>` (only `Error::from_anyhow`).
2. **Cooperative starvation on single-threaded executors.** While the guest
   has continuous progress (e.g. a write loop the host always accepts),
   `Store::run_concurrent`'s future never returns `Pending`, so on a
   current-thread tokio runtime sibling tasks and timers starve — plain
   `#[tokio::test]` deadlocked; `flavor = "multi_thread"` fixed it. The
   omnia runtime is multi-threaded, but seam tests must not assume timers
   fire while a guest is runnable on the same thread.
3. **Guest-side `FutureReader` read has no drop arm** (wit-bindgen 0.60
   panics on an unexpected return code). In practice unreachable because a
   wasmtime host producer that fails traps the guest first, but it hardens
   the rule from the matrix: the reply future is always written. The
   asymmetry is host-specific: a *guest*-created future (`wit_future::new`)
   takes a `default: fn() -> T` that the writer's `Drop` auto-writes, so
   guests cannot drop-without-write at all; the host has no such guard.
4. **`StreamReader`/`FutureReader` on the host must be disposed** (`close`)
   if not transferred to the guest — returning them in the record transfers
   ownership and handles this; error paths must close explicitly.
5. Guest `StreamWriter::write_one` returning `Some(value)` (rejection) and
   `StreamReader::next()` returning `None` (closure) are the guest-visible
   drop signals; both surfaced exactly as the design assumes.
6. **Keep `stream<…>`/`future<…>` anonymous in the WIT.** A named typedef
   (`type calls = stream<tool-call>`) makes wasmtime 47's host bindgen emit
   `HostStream`/`HostFuture` aliases that don't exist in the public API;
   anonymous uses (record fields, params, returns) map cleanly to
   `StreamReader`/`FutureReader`. The 0.2 WIT should inline them as this
   spike does.
7. For backend tasks that *do* need store access, `Accessor::spawn` /
   `AccessorTask` runs them on wasmtime's event loop; this spike's channel
   bridge kept the backend store-free instead, which is the simpler shape
   for omnia's model backends.

## What this means for the canvas plan

- Commit `omnia:model@0.2.0` with the session record as sketched — no
  flattened tuple, no two-call handshake.
- wasi-model's host `create` will mirror [host/src/lib.rs](host/src/lib.rs):
  mint calls/reply inside `accessor.with`, pipe the results reader, and keep
  the backend (genai loop / cursor bridge) behind channels with no store
  coupling.
- Express budgets/deadlines as `error` values written into the reply future
  plus closing the calls stream — never by dropping the reply writer.
- The `omnia-guest` `complete_with` sugar is exactly the spike guest's
  `session_loop` (join! over calls + reply) and needs no executor tricks.

This directory is throwaway; delete it once Phase 3 lands.
