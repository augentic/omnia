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

A guest artifact the deployment names loads in either format, told apart the way wasmtime tells them apart: a raw `.wasm` is validated and compiled by wasmtime and runs inside the sandbox described above, and `omnia compile` output (`.cwasm` / `.bin`) is deserialized as the native code it is. The two differ in what a wrong artifact can do. A hostile `.wasm` is a hostile guest — it can use every host capability the runtime compiled in and its imports allow, but it cannot escape the sandbox. A hostile pre-compiled artifact is arbitrary native code running with host privileges: wasmtime's compatibility check (rejecting artifacts built with mismatched compile-affecting settings) is a compatibility check, *not* an authenticity check, and the runtime does not distinguish the two formats beyond that. What the deployment names is therefore the operator's responsibility, exactly as the manifest is — the CLI's arguments, a manifest's `[[guest]]` paths and packages, and the macro's compiled-in `path:` bytes alike. Which format a source may load is fixed by the source kind, with no key to set or get wrong. Bytes that were in the process before any guest ran — the macro's embedded `path:`, the component `run <component>` names, a programmatic `GuestEntry::new(name, bytes)` — load at boot in either format. A manifest file's `source.path` is read at the guest's first use, while guests are already running, so `omnia compile` output is admitted from it only when the entry pins its `digest`; unpinned, the artifact is refused and raw wasm under the same entry is compiled as usual. A `source.package` is fetched from a registry at first use and admits raw wasm alone, however it hashes — a pin proves the bytes are the ones the entry named, not that they are safe to run as native code, and a registry is not the deployment's build. So the worst a package can name is a hostile guest inside the sandbox, and a `.bin` path is native code the operator vouched for by pinning it. Produce pre-compiled artifacts in your own build pipeline, ship them from immutable or content-addressed storage, and pin every `.bin` path. The same split holds for an embedder's own code: `Runtime::register(id, bytes)` admits raw wasm alone, and a pre-compiled artifact reaches `Runtime::admit` only wrapped in `unsafe { Verified::trusted(bytes) }` — the embedder's word, at the call site, that the bytes came from its own toolchain.

Two further consequences of dynamic loading:

- **Artifacts are read when they load** — at startup for embedded bytes and `run <component>`, at first use for a manifest file's `source.path` or `source.package`, at every `load` for a path a requester names. A guest path can be substituted between manifest construction and that read; prefer immutable or content-addressed artifact locations, especially for `.bin`, and pin a `digest` on any entry whose file could change under you.
- **Startup cost is unbounded by the runtime.** Nothing caps the manifest's guest count or artifact sizes; compilation cost at startup is bounded only by what the manifest names — another reason the manifest is an operator-privilege input.

Note also that guest-to-guest dispatch is a deployment-level grant with no per-guest ACL: every interface outside the runtime's own `wasi:` and `omnia:` namespaces is relayed to whichever guest exports it, the linker is shared, and any guest importing such an interface may call it. What a guest may reach is bounded by which guests the deployment runs — the operator's choice of components — not by a declared list. The namespace predicate is closed: a native host linked under any namespace other than `wasi:` or `omnia:` collides with the relay polyfill at boot rather than serving its interface, so a third-party host lives under `omnia:` or the predicate grows.

## Guest-requested plugin loading (`omnia:plugins/loader`)

A runtime built with omnia's `loader` feature links the loader capability at assembly, letting a guest whose world imports it request that the host admit another component at run time. The design keeps every trust decision host-side:

- **A location in, a handle out — never bytes.** The interface carries a `location` and an optional digest in and a typed handle out; component bytes structurally cannot cross it in either direction, and the requester gains no lifecycle authority (no deregister, no mutation of mounts, hosts, or routes). A location is one of three: `declared(name)` names a guest the deployment's `[[guest]]` list declares; `path(p)` names a component file beneath one of the deployment's mounts; `registry(package, endpoint?)` names an exact `namespace:name@version`. Each resolves to one `Source` inside the deployment's grant, and the host acquires, verifies, validates, compiles, and publishes the bytes it finds there.
- **The deployment's grant bounds every arm.** `Deployment::assemble` fixes the grant at install: the deployment's guest list as the names a `declared` load may name, the runtime's mounts as the roots a `path` may lie beneath, and the `registries` routing a `package` is fetched through. A `declared` name or a `path` selects within those tables and can widen neither; a `registry` load may also name an `endpoint`, which the deployment's routing outranks. A `declared` name outside the list is refused; one not yet loaded goes through the runtime's own first-use seam, exactly as a route or a link call would load it; one already active (loaded at boot, or used earlier) is attested without fetching anything. It takes no digest of its own — the entry carries the pin. A declared name is bound by its entry alone: a `path` or `package` that would register under it is refused before anything is read, whether or not the guest has loaded, so nothing a caller names can seat other bytes under the name for the declared load to attest. A `path` is read through the mount it lies beneath (`.` for a bare relative path, the mount's name as its prefix otherwise), with cap-std refusing any escape, and registers as its file stem. A `package` is fetched by the `RegistryClient` routed by the deployment's `registries` (the macro's `registries: include_str!("wasm-pkg.toml")`, a manifest's `[registries] path`) — package override, then namespace, then `default_registry` — or by the custom `RegistrySource` an embedder selected with `Deployment::registry_source` before assembly, and registers as its reference without the version. A package the deployment routes is served by its route, and an `endpoint` naming another registry is refused rather than followed; a package whose namespace the deployment routes nowhere, under no `default_registry`, is fetched from the `endpoint` the load names, and refused when it names none. Registry egress is therefore bounded by the deployment's routing only as far as that routing reaches: a deployment that admits a loader-importing component it does not author closes the arm itself, with a `default_registry` or by routing every namespace, so that every package a load can name is served by a registry the deployment chose.
- **What a guest can write, it cannot run.** A component loads from a read-only mount alone: a `path` beneath a writable mount is refused before the file is read, so a guest with a writable mount cannot plant a component and have the host compile it. Nor can it plant native code where a first use would deserialize it: a manifest file's `source.path` is read when the guest is first used, so a pre-compiled artifact found there is admitted only when the entry pins the bytes, and a guest that overwrites the file gets its planted artifact refused as unpinned (raw wasm it plants is compiled inside the sandbox, as any raw wasm is). The loader also refuses at assembly a writable mount that shares or nests a read-only mount's directory, since a file written through the one view would load through the other; directories are told apart by identity, not path, so a bind mount or firmlink of a read-only mount's directory is refused as that directory. Two read-only mounts, or two writable ones, may nest freely. State a guest must write lives under its own mount; code lives under a read-only one.
- **Requester-named origins admit raw wasm alone.** A `path` or `package` the requester names is not one of the deployment's `Source`s and never becomes one: its bytes are checked against the load's pin and then admitted through `Verified::wasm`, which refuses a pre-compiled artifact however it hashes, so the worst such a load can name is a hostile guest inside the sandbox. A pre-compiled component is admitted only where the deployment itself vouches for it — embedded bytes, `run <component>`, or a `source.path` the operator pinned.
- **Digest pins are verified before wasmtime sees the bytes.** A `[[guest]]` entry may pin the `digest` its bytes must hash to (`sha256:<hex>`), and a `path` or `registry` load may carry one; either is checked against the acquired bytes *before* any wasmtime validation, at boot or at first use alike. An unpinned load reports the resolved digest on the returned handle (trust-on-first-use), so it can be committed as a pin. A location whose name is already active under other bytes — a path whose stem is a loaded guest, a racing load of changed bytes — is refused rather than re-bound; the same bytes attest.
- **Loaded plugins load as every guest does.** A `declared` load is one more first use of the runtime's guest seam (`Runtime::guest`): the entry's `Source` is read, verified against its pin and the format rule, and admitted exactly as a route or a link call would admit it. A requester-named load takes the shared pin check and `Verified::wasm`, then the same admission seam.
- **Call-site failure, not admission.** Admission does not require a loaded component to export what its caller will import — mixed deployments have host-only handlers beside dispatch targets. A subsequent guest→guest call to a registered guest that exports no interface the caller imports fails at the call site (the target is registered but unlinked). Its imports are bounded by the deployment's linked host set exactly like any late-registered guest.
- **Import-gated by worlds.** The loader links once on the shared linker, but wasmtime wires it only into guests whose world imports `omnia:plugins/loader`. A guest (including a loaded plugin) whose world does not name the import can never reach it; note the converse — a loaded plugin whose world *does* import the loader can itself request loads, from the same grant.

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
- **An unrouted registry namespace is the caller's to route.** A `registry` load whose package the deployment's `registries` routes nowhere — no package override, no namespace mapping, no `default_registry` — is fetched from the `endpoint` the load names: any `host[:port]`, dialled by the host's `RegistryClient` rather than through `WasiHttp` (so a deployment that links no HTTP still has this one egress path), with whatever credentials wasm-pkg resolves for that host (the deployment's `[registry."<host>"]` settings, else the Docker credential store). The loader's reach is the deployment's routing plus, for the namespaces it leaves unrouted, any host a loader-importing guest names; where such a guest is not the operator's own, set a `default_registry` or route every namespace.
- **Backend credentials are host-side.** Guests never see connection strings, but any guest with the interface linked can use the backend's full capability (e.g. every bucket the Redis credential can reach). Scope service credentials to what the deployment needs.
- **Within one interface, granularity is the backend's.** `wasi:keyvalue` doesn't partition buckets per guest; guests in one deployment sharing a backend share its namespace.
- **Writable mounts are real writes.** A writable workspace lent to a model backend can be modified by the model. Review flows should mount read-only and route writes through validated tools.
- **Mount overlap is judged along each mount's own ancestry.** The loader identifies a mount's directory and every directory above it by `(device, inode)`, which catches a second mount point of a code root or of a directory above it. A writable mount that is a second view of a *subdirectory* beneath a read-only mount — a bind mount of `code/out` at `/scratch`, say — presents as an unrelated directory and is not refused; keep such views out of code roots.
- **Denial of service via legitimate traffic** is bounded per invocation, but request admission (rate limiting, auth) is upstream of the runtime.
- **Side channels** (timing, cache) are out of scope, as for most wasm runtimes.

## Defence-in-depth checklist

- [ ] Treat manifests and every artifact they name — pre-compiled ones above all — as trusted operator inputs; never build either from untrusted data
- [ ] Take only raw `.wasm` from less-trusted sources, and run it with minimal hosts and read-only mounts; declare such a guest as a `source.package`, or a `source.path` you leave unpinned, so the runtime itself refuses native code from it; never expect a package or a caller-named load to run `omnia compile` output; feed an embedder's `Runtime::register` raw wasm alone, and reach for `Verified::trusted` only for artifacts your own build produced
- [ ] Pin every `.bin` a manifest's `source.path` names — the pin is what admits it — and every other `[[guest]]` entry by `digest` wherever the artifact is known ahead of time; pass a digest on every `path` or `registry` load whose bytes are known; treat an unpinned load's reported digest as the pin to commit
- [ ] Mount code read-only and state under a mount of its own; the loader refuses a component beneath a writable mount, and a writable mount that overlaps a read-only one, so the two never share a directory
- [ ] Give the loader import only to worlds that genuinely request loads; keep loadable-plugin worlds free of it
- [ ] Set a `default_registry`, or route every namespace, in any deployment whose loader-importing components are not the operator's own; a namespace routed nowhere is fetched from whichever `endpoint` the load names
- [ ] Link only the interfaces each deployment's guests need
- [ ] Mount the minimum directory set, read-only unless writes are required
- [ ] Keep resource ceilings meaningful for the workload (don't blanket-raise timeouts and memory)
- [ ] Scope backend service credentials narrowly; prefer per-deployment credentials
- [ ] For model workloads, prefer read-only workspaces; treat `writable` lends as privileged
- [ ] Run the host container as non-root with a minimal image (see [Deploying Omnia](guides/deployment.md#container-images))
