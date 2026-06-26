# RFC-59: Genai Tool Loop — Working-Tree

> Status: Draft · Order 4 of 10 · **remaining work only** · Depends: the landed
> `wasi-model` boundary, [RFC-55](rfc-55-working-tree.md) · Owns: the genai backend's
> remaining in-process tools inside one `complete`

## Abstract

The genai backend's in-process tool loop already drives a hosted model through tools:
`resolve` (lazy reference resolution via host-mediated dynamic linking,
instance-per-call) and the **repair loop** (invalid candidates are loop inputs, never
guest-visible; the loop is budgeted and terminates clearly) are landed. This RFC now
tracks only the tools that are **not yet built**: the working-tree `read` / `list` /
`write` surface and the host-held session state that backs it.

## Remaining tool surface

Within one `complete`, `GenaiBackend` already advertises and dispatches `resolve` and
`verify` (routing only; profiles are [RFC-60](rfc-60-verify-profiles.md)). The remaining
tools are the working-tree trio, which are **loud stubs** today:

- `read` **/** `list` — inspect the working tree through the capability lent in
`prompt.grants.working-tree`. The model sees bounded tool results, never an OS path or
descriptor.
- `write` — accumulate an edit against the session's base tree. Pending edits live in
host-held state, not guest memory.

These consume the wasi-filesystem working-tree host of
[RFC-55](rfc-55-working-tree.md), which is **not yet built**. Wiring them is gated on
RFC-55; the floor-side host wiring is tracked in [wasi-model-remaining.md](wasi-model-remaining.md) §3, and
this RFC owns the genai-side dispatch and transcript capture for them.

## Remaining session state

One genai session binds the prompt request and expected type, the adapter identity in
scope, the base `revision`, the working-tree capability, accumulated edits, and verify
results. Because guests are instance-per-call, durable session state must live in
`wasi:keyvalue` (keyed by the prompt hash), so a `write` in one tool turn is visible to a
`read` in the next.

Today the genai loop's per-call working state lives in the `complete` future only; the
cross-turn `wasi:keyvalue`-backed state that makes `write` → `read` visibility durable
lands with the working-tree tools above. A leaked in-memory session is a regression.

## Scope

- Tool-call dispatch for `read`, `list`, and `write` inside `GenaiBackend`.
- Host-held cross-turn session state in `wasi:keyvalue` for one genai `complete`.
- Recordable tool transcripts for the working-tree tools, replayable through the
`wasi-model` replay boundary.



## Out of scope

- `resolve` and the repair loop — landed.
- The floor-side host wiring of the working-tree capability; see
[wasi-model-remaining.md](wasi-model-remaining.md) §3.
- Verify profile definitions and sandboxing; see [RFC-60](rfc-60-verify-profiles.md).
- Backend catalogue and routing; see [RFC-58](rfc-58-model-router-slm.md).



## Acceptance criteria

1. `read`, `list`, and `write` operate through bounded working-tree tools; the model
  holds no descriptor or OS path.
2. Session state survives callbacks through host-held `wasi:keyvalue` storage, not guest
  memory.
3. The working-tree tool transcripts are recordable and replayable through the
  `wasi-model` replay boundary.



## Risks and invariants

- **Handles, not corpora.** The prompt carries handles and reference ids; the model pulls
only what it needs.
- **Instance-per-call.** Every guest callback lands in a fresh instance and never
recursively re-enters the caller.
- **Budgeted repair.** The loop must fail clearly rather than silently continuing or
returning unvalidated output.

