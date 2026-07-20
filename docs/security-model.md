# Security Model

Omnia exists to run untrusted or semi-trusted code — including agent-generated code — safely alongside real infrastructure. This page explains what the sandbox guarantees, how capabilities are granted, and, just as importantly, what the runtime does *not* protect against.

## The trust boundary

The boundary sits between the **guest** (WebAssembly, untrusted) and the **host** (native, trusted). Everything a guest can do, it does through a WASI interface the host explicitly linked; everything else is unreachable by construction:

- **No ambient filesystem.** A guest sees only the directories the host preopened via mounts, read-only unless marked writable.
- **No ambient network.** A guest cannot open sockets. Outbound HTTP exists only if the host linked `WasiHttp`, and then only through the host's client.
- **No process, clock, or environment escape.** Environment variables reach the guest only through `wasi:cli` argv/env the host chooses to forward, or `wasi:config`.
- **Memory isolation.** WebAssembly linear memory is bounds-checked; a guest cannot read host memory or another guest's.

Capability granting is therefore the host author's main security decision: the `hosts:` map in `runtime!` *is* the guest's permission set. A guest that only needs key-value storage should run in a host that links only `WasiKeyValue` (plus a trigger).

## Deployment inputs are trusted

The manifest sits on the *host* side of the trust boundary. Whether it arrives as an `omnia.toml` file or is assembled programmatically as an `omnia::Manifest`, it chooses which guest artifacts load, which host directories mount (including `writable`), and which interfaces the host dispatches between guests. Manifest validation is structural (at least one guest, unique ids, in-process transport) — it is **not** authorization. Never build a manifest from untrusted data.

Guest artifacts split into two trust classes:

- **Raw `.wasm`** is validated and compiled by wasmtime and runs inside the sandbox described above. It is the format to accept from less-trusted sources — the artifact can use every host capability the runtime compiled in and its imports allow, but it cannot escape the sandbox.
- **Pre-compiled `.bin`** is native code, loaded via wasmtime's `unsafe` deserialization. Wasmtime's compatibility check (rejecting artifacts built with mismatched compile-affecting settings) is *not* an authenticity check: a malicious `.bin` is arbitrary native code running with host privileges. Load `.bin` only from trusted, immutable storage — signed or digest-pinned artifacts your build pipeline produced.

Two further consequences of dynamic loading:

- **Artifacts are read at startup.** A guest path can be substituted between manifest construction and load; prefer immutable or content-addressed artifact locations, especially for `.bin`.
- **Startup cost is unbounded by the runtime.** Nothing caps the manifest's guest count or artifact sizes; compilation cost at startup is bounded only by what the manifest names — another reason the manifest is an operator-privilege input.

Note also that `link` allow-lists flatten onto the one shared linker: an interface linked for *any* guest is wired for the *whole* deployment. Treat `link` (per-guest, top-level, or CLI `--link`) as a deployment-level grant, not a per-guest ACL.

## Isolation between requests and guests

Every invocation runs in a **fresh instance in its own store**, torn down afterwards. Consequences:

- Nothing persists in guest memory between requests — no request can read another's data through the guest heap, and a compromised request state dies with the instance.
- In multi-guest deployments, guests share an engine and linker but never an instance or store. They can interact only through host-mediated dispatch, and only along interfaces named in a guest's `link` allow-list, with nesting bounded by `MAX_DISPATCH_DEPTH`.
- The runtime core treats guest ids and interface names as opaque strings — no domain knowledge, no special cases a guest could exploit by name (the glossary's [Law 2](glossary.md#law-2)).

State that must persist lives behind a WASI interface (keyvalue, sql, blobstore, ...) where the host controls it.

## Resource containment

Sandboxing without resource limits is denial-of-service waiting to happen. Each invocation is bounded by:

| Limit | Variable | Default |
| ----- | -------- | ------- |
| Wall-clock time | `GUEST_TIMEOUT_MS` | 30 s |
| Linear memory | `MAX_MEMORY_BYTES` | 256 MiB |
| Instruction budget | `MAX_FUEL` | off (`0`) |
| Preemption granularity | `EPOCH_TICK_MS` | 10 ms |
| Dispatch nesting | `MAX_DISPATCH_DEPTH` | 8 |

Epoch interruption preempts CPU-bound guests, so an infinite loop cannot hold an executor thread past the timeout. Pool ceilings (`POOL_MAX_INSTANCES` and friends) cap aggregate resource use across concurrent requests.

## Filesystem: mounts

Mounts are the only filesystem doorway ([details](guides/multi-guest-deployments.md#mounts-giving-guests-a-workspace)):

- Explicit: a `[[mount]]` in the manifest or `--mount` on the command line. No mount, no filesystem.
- **Read-only by default**; writes require an explicit `writable`.
- Scoped: the preopen is rooted at the mounted directory. Paths cannot traverse above it.
- Shared: mounts preopen into *every* guest in a deployment — the mount set should be the union of what the deployment's guests legitimately need, kept minimal.

## Model completions: lending, not granting ambient access

The `omnia:model` design extends capability thinking to LLM backends, which are effectively untrusted executors:

- The backend gets **no ambient access**. It can touch a filesystem tree only if the guest lends one through `grants.workspace` — and that lend is a typed `wasi:filesystem` descriptor borrow from the guest's own preopen table, not a path string or integer handle a guest could forge. The host resolves it back to an authorized mount by identity.
- Tools the model may call (`resolve`, `read`, `list`, `write`, `verify`) are **injected by the host** from the grants; backends execute them by calling back through the host, so every tool invocation passes host-side checks. Guests cannot impersonate these tools (reserved names are rejected).
- The **answer is validated by the host** against the requested format before the guest sees it — a backend cannot smuggle unvalidated output past the gate.
- Budget errors (`budget-exhausted`) bound runaway tool loops; the cursor backend additionally kills its spawned agent on timeout.

The net effect: a prompt-injected or misbehaving model session is confined to the lent workspace and the granted tools, exactly as a guest is confined to its linked interfaces.

## What Omnia does not protect against

Honest limits, so you can layer the right controls on top:

- **Outbound HTTP is coarse.** If `WasiHttp` is linked, the guest can request any URL the host can reach — there is no per-guest URL allow-list today. Network egress policy belongs at the infrastructure layer (network policies, egress proxies).
- **Backend credentials are host-side.** Guests never see connection strings, but any guest with the interface linked can use the backend's full capability (e.g. every bucket the Redis credential can reach). Scope service credentials to what the deployment needs.
- **Within one interface, granularity is the backend's.** `wasi:keyvalue` doesn't partition buckets per guest; guests in one deployment sharing a backend share its namespace.
- **Writable mounts are real writes.** A writable workspace lent to a model backend can be modified by the model. Review flows should mount read-only and route writes through validated tools.
- **Denial of service via legitimate traffic** is bounded per invocation, but request admission (rate limiting, auth) is upstream of the runtime.
- **Side channels** (timing, cache) are out of scope, as for most wasm runtimes.

## Defence-in-depth checklist

- [ ] Treat manifests and pre-compiled `.bin` artifacts as trusted operator inputs; never build either from untrusted data
- [ ] Accept only raw `.wasm` from less-trusted sources, and run it with minimal hosts and read-only mounts
- [ ] Link only the interfaces each deployment's guests need
- [ ] Mount the minimum directory set, read-only unless writes are required
- [ ] Keep resource ceilings meaningful for the workload (don't blanket-raise timeouts and memory)
- [ ] Scope backend service credentials narrowly; prefer per-deployment credentials
- [ ] For model workloads, prefer read-only workspaces and closed `verify` profiles; treat `writable` lends as privileged
- [ ] Run the host container as non-root with a minimal image (see [Deploying Omnia](guides/deployment.md#container-images))
