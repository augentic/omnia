# Security Model

Omnia exists to run untrusted or semi-trusted code — including agent-generated code — safely alongside real infrastructure. This page explains what the sandbox guarantees, how capabilities are granted, and, just as importantly, what the runtime does *not* protect against.

## The trust boundary

The boundary sits between the **guest** (WebAssembly, untrusted) and the **host** (native, trusted). Everything a guest can do, it does through a WASI interface the host explicitly linked; everything else is unreachable by construction:

- **No ambient filesystem.** A guest sees only the directories the host preopened via mounts, read-only unless marked writable.
- **No ambient network.** A guest cannot open sockets. Outbound HTTP exists only if the host linked `WasiHttp`, and then only through the host's client.
- **No process, clock, or environment escape.** Environment variables reach the guest only through `wasi:cli` argv/env the host chooses to forward — the process environment, with `RUST_LOG` set to the runtime's tracing level — or `wasi:config`.
- **Memory isolation.** WebAssembly linear memory is bounds-checked; a guest cannot read host memory or another guest's.

Capability granting is therefore the host author's main security decision: the `hosts:` map in `runtime!` *is* the guest's permission set. A guest that only needs key-value storage should run in a host that links only `WasiKeyValue` (plus a trigger).

## Deployment inputs are trusted

The manifest sits on the *host* side of the trust boundary. Whether it arrives as an `omnia.toml` file or is assembled programmatically as an `omnia::Manifest`, it chooses which guest artifacts load, which host directories mount (including `writable`), and which interfaces the host dispatches between guests. Manifest validation is structural (at least one guest, unique ids, in-process transport) — it is **not** authorization. Never build a manifest from untrusted data.

A guest artifact the deployment names loads in either format, told apart the way wasmtime tells them apart: a raw `.wasm` is validated and compiled by wasmtime and runs inside the sandbox described above, and `omnia compile` output (`.cwasm` / `.bin`) is deserialized as the native code it is. The two differ in what a wrong artifact can do. A hostile `.wasm` is a hostile guest — it can use every host capability the runtime compiled in and its imports allow, but it cannot escape the sandbox. A hostile pre-compiled artifact is arbitrary native code running with host privileges: wasmtime's compatibility check (rejecting artifacts built with mismatched compile-affecting settings) is a compatibility check, *not* an authenticity check, and the runtime does not distinguish the two formats beyond that. What the deployment names is therefore the operator's responsibility, exactly as the manifest is — the CLI's arguments, a manifest's `[[guest]]` paths, the macro's compiled-in `path:` bytes, and the loader's on-demand sources alike. Produce pre-compiled artifacts in your own build pipeline, ship them from immutable or content-addressed storage, and pin a `digest` on any entry whose file could change under you. Where an entry is declared from input the deployment does not author — a path and a pin read from a project's own configuration, say — the pin proves only that the bytes are the ones that input named, so mark the entry `wasm_only`: a pre-compiled artifact is then refused however it hashes, and the worst such an entry can name is a hostile guest inside the sandbox.

Two further consequences of dynamic loading:

- **Artifacts are read when they load** — at startup for a boot guest, at first `load` for an `on_demand` one. A guest path can be substituted between manifest construction and that read; prefer immutable or content-addressed artifact locations, especially for `.bin`, and pin a `digest` on any entry whose file is reachable through a writable mount.
- **Startup cost is unbounded by the runtime.** Nothing caps the manifest's guest count or artifact sizes; compilation cost at startup is bounded only by what the manifest names — another reason the manifest is an operator-privilege input.

Note also that guest-to-guest dispatch is a deployment-level grant with no per-guest ACL: every interface outside the runtime's own `wasi:` and `omnia:` namespaces is relayed to whichever guest exports it, the linker is shared, and any guest importing such an interface may call it. What a guest may reach is bounded by which guests the deployment runs — the operator's choice of components — not by a declared list. The namespace predicate is closed: a native host linked under any namespace other than `wasi:` or `omnia:` collides with the relay polyfill at boot rather than serving its interface, so a third-party host lives under `omnia:` or the predicate grows.

## Guest-requested plugin loading (`omnia:plugins/loader`)

A runtime built with omnia's `loader` feature links the loader capability at assembly, letting a guest whose world imports it request that the host admit another component at run time. The design keeps every trust decision host-side:

- **Name-only, byte-free.** The interface carries a name in and a typed handle out — component bytes, paths, and registry endpoints structurally cannot cross it in either direction. A guest never supplies or locates code: it names a guest the deployment already declares, and the host acquires the bytes from that entry's declared source. Validation, compilation, and publication all happen host-side; the requester gains no lifecycle authority (no deregister, no mutation of mounts, hosts, or routes).
- **The guest list is the allow-list.** Every component that may ever run is a `[[guest]]` entry (the macro's `guests:`); one marked `on_demand` is admitted when first named rather than at boot, from the source the entry declares — a file path, bytes embedded in the host binary, or an exact registry package. `Deployment::assemble` installs the manifest's on-demand entries as the loader's table, fixed at install; a name outside it is refused, and a name already active (declared at boot, or admitted earlier) is attested without fetching anything. Mounts play no part: a filesystem grant is a data grant, never a code-admission root, so a guest with a writable mount cannot plant a component and have the host compile it. A package source is fetched by the `RegistryClient` routed by the deployment's `registries` configuration (the macro's `registries: include_str!("wasm-pkg.toml")`, a manifest's `[registries] path`) — package override, then namespace, then `default_registry`; a package routed nowhere is refused before any registry is dialled — or by the custom `RegistrySource` an embedder selected with `Deployment::registry_source` before assembly. Registry egress is therefore bounded by the operator's own manifest: the packages it names and the registries it routes them to.
- **Digest pins are operator-anchored.** A `[[guest]]` entry may pin the `digest` its bytes must hash to (`sha256:<hex>`); the pin is verified against the acquired bytes *before* any wasmtime validation, at boot or on demand alike. An unpinned entry reports the resolved digest on the returned handle (trust-on-first-use), so it can be committed as a pin. Pin every on-demand `source.path` whose file lives under a directory the deployment also mounts writable: the file is read fresh at first load, so without a pin a guest that can write the mount could replace the component between boot and load.
- **Loaded plugins load as boot guests do.** One `Source` per entry carries the same checks to both paths: the bytes an entry's source produces are verified against its digest pin and its `wasm_only` mark before wasmtime sees them, then loaded in whichever format they are, raw wasm or pre-compiled. A pre-compiled on-demand entry the operator ships is one to pin; an entry the deployment declares from input it does not author is one to mark `wasm_only`, so a pre-compiled artifact planted under the declared path is refused however it hashes.
- **Call-site failure, not admission.** Admission does not require a loaded component to export what its caller will import — mixed deployments have host-only handlers beside dispatch targets. A subsequent guest→guest call to a registered guest that exports no interface the caller imports fails at the call site (the target is registered but unlinked). Its imports are bounded by the deployment's linked host set exactly like any late-registered guest.
- **Import-gated by worlds.** The loader links once on the shared linker, but wasmtime wires it only into guests whose world imports `omnia:plugins/loader`. A guest (including a loaded plugin) whose world does not name the import can never reach it; note the converse — a loaded plugin whose world *does* import the loader can itself request loads.

## Isolation between requests and guests

Every invocation runs in a **fresh instance in its own store**, torn down afterwards. Consequences:

- Nothing persists in guest memory between requests — no request can read another's data through the guest heap, and a compromised request state dies with the instance.
- In multi-guest deployments, guests share an engine and linker but never an instance or store. They can interact only through host-mediated dispatch, along the interfaces they import and export outside the runtime's own namespaces, with nesting bounded by `MAX_DISPATCH_DEPTH`.
- The runtime core treats guest ids and interface names as opaque strings — no domain knowledge, no special cases a guest could exploit by name (the glossary's [Law 2](glossary.md#law-2)).

State that must persist lives behind a WASI interface (keyvalue, sql, blobstore, ...) where the host controls it.

## Resource containment

Sandboxing without resource limits is denial-of-service waiting to happen. Each invocation is bounded by:

| Limit | Variable | Default |
| ----- | -------- | ------- |
| Wall-clock time | `GUEST_TIMEOUT_MS` | 30 s (server invocations and server-rooted link hops; a command-mode chain, link hops included, is uncapped) |
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
- Host-injected tools (`read`, `list`, `write`) are **served and bounded by the host**: the names are reserved, and the backend can only execute them through the host's `ToolHost`, which requires the workspace grant and enforces read/listing bounds (genai advertises `read`/`list` when a workspace is lent; `write` stays unadvertised). Guest-declared function tools are answered by the guest itself over the completion session's streams; the backend forwards each call through the host, which enforces the declared-name allowlist, size cap, and timeout. Guests cannot impersonate the injected tools (reserved names are rejected).
- The **answer is validated by the host** against the requested format before the guest sees it — a backend cannot smuggle unvalidated output past the gate.
- Session limits bound runaway tool loops: a call budget and per-call timeout (`budget-exhausted`) and a per-result size cap (`tool-failed`); the cursor backend additionally cancels its bridge-managed run (`CancelRun`) on timeout or when a session tool call fails hard. Its tool-callback channel is loopback-only and bearer-authenticated, and callbacks route through the same host-enforced `ToolHost::call_tool` path as genai's session tools.

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

- [ ] Treat manifests and every artifact they name — pre-compiled ones above all — as trusted operator inputs; never build either from untrusted data
- [ ] Take only raw `.wasm` from less-trusted sources, and run it with minimal hosts and read-only mounts; mark `wasm_only` every `[[guest]]` entry whose path or pin comes from input the deployment does not author, so the runtime enforces it
- [ ] Pin every `[[guest]]` entry by `digest` wherever the artifact is known ahead of time — always for an on-demand file under a writable mount; treat an unpinned load's reported digest as the pin to commit
- [ ] Give the loader import only to worlds that genuinely request loads; keep loadable-plugin worlds free of it
- [ ] Link only the interfaces each deployment's guests need
- [ ] Mount the minimum directory set, read-only unless writes are required
- [ ] Keep resource ceilings meaningful for the workload (don't blanket-raise timeouts and memory)
- [ ] Scope backend service credentials narrowly; prefer per-deployment credentials
- [ ] For model workloads, prefer read-only workspaces; treat `writable` lends as privileged
- [ ] Run the host container as non-root with a minimal image (see [Deploying Omnia](guides/deployment.md#container-images))
