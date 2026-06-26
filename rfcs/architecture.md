# Specify on Omnia — Remaining Architecture

> Status: This is the standing architecture, trimmed to the **work that remains**. The
> generic runtime floor, the multi-guest registry, host-mediated dynamic linking (over the
> in-process transport), and the `wasi-model` judgment boundary with its frontier
> (`omnia-genai`), spawned-agent (`omnia-cursor`), and replay backends have **landed**.
> What follows is the direction still being built toward.

## The core idea

The architecture boils down to a single concept:

> The "Specify" CLI is Omnia compiled with Specify-specific Wasm guests.

The runtime hosts Wasm guests and satisfies a fixed vocabulary of typed effects; it holds
no domain, workflow, or model knowledge. Specify's behaviour — orchestrating the
workflow, extracting from sources, building for targets, and development tooling — lives
in the guests and in the backends bound behind Omnia's host interfaces.

Context comes from artifacts, not conversational history: each model evaluation is
self-contained, handed one whole brief and a typed tool surface scoped to concrete
artifacts, never an accumulated chat transcript.

## The four laws

These principles govern every remaining design decision:

1. **Typed boundaries**: WIT records for data, WIT interfaces for effects. Untyped text is
   not passed across boundaries.
2. **The runtime floor only knows effects**: Omnia core doesn't know about workflows,
   adapters, or models — only how to host guests and satisfy typed effects. Which backend
   satisfies an interface is deployment configuration the floor never sees.
3. **Determinism by default, judgment by exception**: control flow lives in deterministic
   guest code; `wasi-model.complete` returns a typed, validated decision that steers the
   next deterministic step. Models do not guess control flow.
4. **Laziness is key**: handles (file paths, reference ids) cross boundaries instead of
   corpora, keeping context windows small and operations scalable.

## What remains to be built

### The working tree ([RFC-55](rfc-55-working-tree.md))

A `build` generates a slice *into a pre-existing project*, reading existing code and
writing changes back in place. Modeling that tree as a bare path string is the one thing
that pins an operation to a single machine, so the contract models it as a
host-materialized **working tree** capability instead — a git-aware `wasi:filesystem`
backend that materializes a content-addressed **base revision** onto whichever node runs
the operation.

The capability exposes two faces:

- a `wasi:filesystem` **descriptor**, for deterministic guest code that reads or validates
  the tree through capability-scoped handles;
- a host-reported node-local **`local-path`**, for the one consumer that cannot hold a
  descriptor: the filesystem-capable spawned-agent backend. An absent path is a clean
  capability signal that no real local tree exists on this node.

The agent's read-modify-write loop is quarantined between two portable boundaries: a
host-materialized tree on the way in, and a content-addressed **change-set** (adds,
modifies, deletes against the base revision) on the way out — which is what lets `build`
and `merge` run on different nodes. This host is **not yet built**; it gates the
`wasi-model` working-tree tools (`read` / `list` / `write`) and the spawned agent's direct
access (today behind an `OMNIA_WORKSPACE` config stopgap).

### Verify profiles ([RFC-60](rfc-60-verify-profiles.md))

When a prompt calls for it the model emits `verify(<check>)`; the backend runs a vetted,
sandboxed profile and feeds a severity-tiered `report` back so the model can repair and
re-verify. The floor currently **routes** a verify request against `grants.verify`; the
profile definitions, sandboxing, severity mapping, and the capability signaling for nodes
that cannot verify are unbuilt.

### Lifecycle of an operation

Once the working tree and verify profiles land, a `build` flows end-to-end:

1. The workflow guest resolves the slice to a base revision and asks the host to
   materialize the slice's working tree.
2. It runs any deterministic setup (a `tool` adapter export, via host-mediated linking).
3. For the judgment leg it calls `wasi-model.complete` with a structured `prompt`.
4. The model backend drives the model; `read` / `list` scan existing code through the
   working tree, `write` accumulates an edit. (`resolve` of the adapter's reference shelf
   is already wired.)
5. The model emits `verify(<check>)`; the backend runs the sandboxed profile and feeds the
   severity-tiered `report` back; the model repairs and re-verifies.
6. `complete` returns the validated, typed answer to the guest.
7. The host extracts the resulting mutations as a content-addressed `change-set`; the
   guest requests the lifecycle `transition` effect.

Steps 4–7 depend on the working tree (RFC-55) and verify profiles (RFC-60); step 7's
lifecycle `journal`/`transition` effect is part of the workflow-as-guests move below.

### Workflow and development tooling as guests ([RFC-57](rfc-57-specify-guests.md))

The workflow (`plan`, `execute`) and the development tooling should run on the runtime as
peers to every adapter, reaching the world only through host interfaces. The runtime can
host many guests today; moving Specify's own logic onto it — and binding the lifecycle
`journal` effect (the durable lifecycle log and its legal transitions) — is unbuilt.

### Model backends: router and SLM ([RFC-58](rfc-58-model-router-slm.md), [RFC-18](future/rfc-18-slm.md))

Swapping the model backend is how deployment modes are chosen. The frontier, spawned-agent,
and replay backends are landed. What remains:

- **Router** — selects a backend per call by difficulty, deployment mode, or an abstract
  cost / quality hint, never a vendor model id supplied by a guest.
- **Small local model** — a local SLM for narrow, high-volume transformations, with
  constrained decoding for valid typed reports.

### The typed contract and a vertical proof ([RFC-51](rfc-51-adapter-wit.md), [RFC-52](rfc-52-effect.md), [RFC-54](rfc-54-orchestration.md))

The Specify-side contract is consumer work built *on* the landed floor:

- **Typed contract (RFC-51)** — the versioned `augentic:specify` WIT package: records,
  per-axis `source` / `target` interfaces, the `references` shelf, and the worlds.
- **Effect map (RFC-52)** — naming the typed effects and their owners (`wasi:filesystem`,
  `wasi:keyvalue`, lifecycle, `references`, `wasi-model`).
- **Vertical operation proof (RFC-54)** — one deterministic `tool` operation and one
  judgment operation through `wasi-model.complete`, proving the path before the workflow
  moves onto the runtime.

### Cluster transports

Inter-guest dispatch and inbound routing run over the in-process wRPC carrier today.
Carrying the *same* dispatch across a cluster (UDS, then NATS / QUIC) so "desktop → cloud"
is a transport swap rather than a code change is the remaining transport work; see
[guest-registry-transports.md](guest-registry-transports.md).

### CLI bootstrapping: the OCI puller

The runtime acquires embedded core guests via `include_bytes!` and local guests by path
today. **Config-driven resolution** of adapters and third-party guests via digest-pinned
references from an OCI store into a local cache is designed as a `GuestSource` seam but the
puller itself is unbuilt.

## The incremental path (remaining stages)

Each stage is independently valuable and forward-compatible on the same typed contract:

- **S1 · Typed contract** ([RFC-51](rfc-51-adapter-wit.md)) — the `augentic:specify` WIT
  package, with host bindings.
- **S2 · Effect map** ([RFC-52](rfc-52-effect.md)) — the typed effects named and assigned
  owners.
- **S3 · Vertical operation proof** ([RFC-54](rfc-54-orchestration.md)) — one
  deterministic and one judgment operation, proving the path.
- **S4 · Working-tree backend** ([RFC-55](rfc-55-working-tree.md)) — the git-aware
  `wasi:filesystem` backend so `build` and `merge` can run on different nodes.
- **S4 · Verify profiles** ([RFC-60](rfc-60-verify-profiles.md)) — closed verification
  profiles, sandboxing, report mapping, and capability signaling.
- **S4 · Workflow (and development) as guests** ([RFC-57](rfc-57-specify-guests.md)) — the
  workflow and development tooling run on the runtime like every adapter.
- **Parallel · Model backends** ([RFC-58](rfc-58-model-router-slm.md)) — the router and SLM
  backends behind `wasi-model`.

## Key trade-offs

- **Omnia is the sole runtime**: the *same* binary and guests run from desktop to cloud
  with only backends swapping — at the cost of a hard dependency on Omnia's host surface.
- **Model evaluation needs egress (or a local model) at** `complete` **time**: the replay
  backend covers CI, and a local SLM backend (above) covers air-gapped runs.
