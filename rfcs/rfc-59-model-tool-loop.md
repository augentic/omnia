# RFC-59: Model Tool Loop

> Status: Draft · Order 4 of 10 · Stage S3 · Depends: [RFC-51](rfc-51-adapter-wit.md), [RFC-53](rfc-53-wasi-model.md) · Coordinates with: [RFC-56](rfc-56-runtime-move.md) · Enables: [RFC-54](rfc-54-orchestration.md), [RFC-60](rfc-60-verify-profiles.md) · Owns: tool-call dispatch inside one `eval`

## Abstract

The `wasi-model` core boundary ([RFC-53](rfc-53-wasi-model.md)) says a guest can ask for judgment with `eval`. This RFC defines what happens inside a judgment session when the backend drives a model through typed tools: lazy reference resolution, working-tree reads and writes, session state, answer repair, and the callback path into adapter reference shelves.

## The tool surface

Within one `eval`, the backend may expose these tools to the model:

- **`resolve(reference)`** — follow a brief's internal reference. The backend selects the adapter whose brief is being evaluated, instantiates a fresh guest, and calls its exported `references` shelf ([RFC-51](rfc-51-adapter-wit.md)) through host-mediated dynamic linking ([RFC-56](rfc-56-runtime-move.md)).
- **`read` / `list`** — inspect the working tree through the capability the host made available for this session. The model sees bounded tool results, not an OS path or descriptor.
- **`write`** — accumulate an edit against the session's base tree. The backend stores pending edits in host-held state, not in guest memory.
- **`verify(check)`** — request a closed verification profile. The profile definitions, sandboxing, and report mapping are owned by [RFC-60](rfc-60-verify-profiles.md).

A filesystem-capable spawned-agent backend ([RFC-58](rfc-58-model-backends.md)) may own its own tool loop and read / write through the `local-path` lent by the working-tree backend ([RFC-55](rfc-55-working-tree.md)). It must still return a validated typed answer through the same [RFC-53](rfc-53-wasi-model.md) boundary and remain recordable.

## Session state

One model session binds:

- the prompt request and expected answer type;
- the adapter identity whose brief and reference shelf are in scope;
- the base `revision`;
- the working-tree capability or local path, if available;
- accumulated edits;
- verify results used by the repair loop.

Because guests are instance-per-call, durable session state lives in `wasi:keyvalue` or another host-held backend. A leaked in-memory session is a regression.

## Repair loop

The backend drives the model until one of these terminal states occurs:

- The answer validates against the requested type and returns through `eval`.
- A tool call fails with a typed, non-repairable error.
- The configured iteration, token, time, or verification budget is exhausted.
- The backend records a failure answer for replay diagnostics.

Invalid answer candidates are repair-loop inputs, not guest-visible answers.

## Scope

- Tool-call dispatch for `resolve`, `read`, `list`, `write`, and `verify`.
- Host-held session state for one `eval`.
- Lazy adapter reference resolution through the `references` shelf.
- Repair-loop convergence and failure semantics.
- Recordable tool transcripts for replay.

## Out of scope

- The `eval` host boundary and backend trait; see [RFC-53](rfc-53-wasi-model.md).
- Verify profile definitions and sandboxing; see [RFC-60](rfc-60-verify-profiles.md).
- Backend catalogue and routing; see [RFC-58](rfc-58-model-backends.md).

## Acceptance criteria

1. `resolve` reaches the selected adapter's `references` shelf by host-mediated dynamic linking and instance-per-call execution.
2. `read`, `list`, and `write` operate through bounded working-tree tools; the model holds no descriptor or OS path.
3. Session state survives callbacks through host-held storage, not guest memory.
4. Invalid answer candidates enter the repair loop and never return directly to the guest.
5. Tool transcripts are recordable and replayable through the [RFC-53](rfc-53-wasi-model.md) replay boundary.

## Risks and invariants

- **Handles, not corpora.** The prompt carries handles and reference ids; the model pulls only what it needs.
- **Adapter-local prose.** References resolve through the adapter's own shelf, not through a host preopen of adapter prose.
- **Instance-per-call.** Every guest callback lands in a fresh instance and never recursively re-enters the caller.
- **Budgeted repair.** The loop must fail clearly rather than silently continuing or returning unvalidated output.
