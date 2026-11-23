# Security Model

Omnia exists to run untrusted or semi-trusted code — including agent-generated code — safely alongside real infrastructure. This page explains what the sandbox guarantees, how capabilities are granted, and, just as importantly, what the runtime does *not* protect against.

## The trust boundary

The boundary sits between the **guest** (WebAssembly, untrusted) and the **host** (native, trusted). Everything a guest can do, it does through a WASI interface the host explicitly linked; everything else is unreachable by construction:

- **No ambient filesystem.** A guest sees only the directories the host preopened via mounts, read-only unless marked writable. A guest under grants sees only the mounts its `mounts` list names.
- **No ambient network.** A guest cannot open sockets. Outbound HTTP exists only if the host linked `WasiHttp`, and then only through the host's client.
- **No process, clock, or environment escape.** A guest's `wasi:cli` environment holds the runtime's own `RUST_LOG` (its tracing filter), `NO_COLOR` when the console is plain, and the host variables the deployment names in `[env] inherit` or with `run --env`. A guest under grants sees only those its `env` list names. Nothing else of the process environment reaches it through `wasi:cli`. `omnia:config` is a separate doorway that answers from its backend, and `ConfigDefault` serves only the host variables `CONFIG_KEYS` names.
- **Memory isolation.** WebAssembly linear memory is bounds-checked; a guest cannot read host memory or another guest's.

Capability granting happens at two levels. The `hosts:` map in `runtime!` is the deployment's permission set: no guest reaches an interface the runtime does not link. A guest's grants narrow that set for the one guest (see [Per-guest grants](#per-guest-grants)). A guest that only needs key-value storage runs in a host that links only `WasiKeyValue` (plus a trigger). In a deployment whose other guests need more, it runs under an `interfaces` list naming `omnia:keyvalue` beside what its SDK imports.

## Per-guest grants

A deployment's guests share one linker, one set of backends, and one set of mounts, so a guest without grants reaches everything the deployment has. A grants table narrows that for one guest. It is `[guest.grants]` in a manifest, `grants: { .. }` on a `runtime!` guest, and `GuestEntry::grants` in code:

```toml
[[guest]]
name = "orders"
source.path = "./orders.wasm"

[guest.grants]
env        = ["ORDERS_*"]                    # of what [env] inherit passes
mounts     = ["data"]                        # [[mount]] names
interfaces = [
    "wasi:cli", "wasi:clocks", "wasi:random", "omnia:otel",  # what every omnia-sdk guest imports
    "wasi:filesystem",                                       # to use its mount
    "omnia:keyvalue", "acme:pricing",
]
```

A guest with no table reaches everything, and startup logs its name. In a table, an omitted list grants nothing, `interfaces` included. An entry is a whole name, or a prefix that ends in `*`.

Each list is held where the runtime reaches what it names:

| List | Held at | A refusal |
| ---- | ------- | --------- |
| `env` | the guest's `wasi:cli` environment | the variable is absent |
| `mounts` | the guest's preopens, and the mounts a model workspace or `omnia:vcs` resolves | the mount is absent |
| `interfaces` | admission, at boot and at first use | the guest is refused, naming every package outside the list |

Some of the rules behind those rows:

- `interfaces` decides whether a guest has a package at all. A guest has all of `omnia:keyvalue`, `omnia:vcs`, or `wasi:http`, or none of it. An import with no function and no resource, such as a `types` interface, needs no entry. Every guest built on `omnia-sdk` imports `wasi:cli`, `wasi:clocks`, `wasi:random`, and `omnia:otel`, so a list without them refuses the guest. A linked package counts, as `acme:pricing` does above.
- Name each package rather than writing a prefix. `wasi:*` grants `wasi:http`, and with it outbound requests, and `wasi:sockets`. `omnia:*` grants `omnia:plugins`, the loader.
- `env` narrows `wasi:cli` alone. Whether a guest has `omnia:config` is its `interfaces`' decision, and `ConfigDefault` serves every guest that has it the variables `CONFIG_KEYS` names.
- Startup checks every table against the deployment. An `env` entry that reaches no variable `inherit` passes, or a `mounts` entry that names no mount, fails startup rather than granting nothing.
- An embedder's `Runtime::register` and `Runtime::admit` take the grants their guest runs under, checked the same way. A name already active attests on the same bytes and the same grants alone, and other grants are `denied`.
- Grants hold what a guest imports. What it exports, and who calls it, they leave alone.

## Deployment inputs are trusted

The manifest sits on the *host* side of the trust boundary. Whether it arrives as an `omnia.toml` file or is assembled programmatically as an `omnia::Manifest`, it chooses which guest artifacts load, which host directories mount (including `writable`), which interfaces the host dispatches between guests, and what each guest's grants reach. Manifest validation is structural (at least one guest, unique ids, well-formed grants, in-process transport) — it is **not** authorization. Never build a manifest from untrusted data.

A guest artifact the deployment names loads in either format, told apart the way wasmtime tells them apart: a raw `.wasm` is validated and compiled by wasmtime and runs inside the sandbox described above, and `omnia compile` output (`.cwasm` / `.bin`) is deserialized as the native code it is. The two differ in what a wrong artifact can do. A hostile `.wasm` is a hostile guest — it can use every host capability the runtime compiled in and its imports allow, but it cannot escape the sandbox. A hostile pre-compiled artifact is arbitrary native code running with host privileges: wasmtime's compatibility check (rejecting artifacts built with mismatched compile-affecting settings) is a compatibility check, *not* an authenticity check, and the runtime does not distinguish the two formats beyond that. What the deployment names is therefore the operator's responsibility, exactly as the manifest is — the CLI's arguments, a manifest's `[[guest]]` paths and packages, and the macro's compiled-in `path:` bytes alike. Which format a source may load is fixed by the source kind, with no key to set or get wrong. Bytes that were in the process before any guest ran — the macro's embedded `path:`, the component `run <component>` names, a programmatic `GuestEntry::new(name, bytes)` — load at boot in either format. A manifest file's `source.path` is read at the guest's first use, while guests are already running, so `omnia compile` output is admitted from it only when the entry pins its `digest`; unpinned, the artifact is refused and raw wasm under the same entry is compiled as usual. A `source.package` is fetched from a registry at first use and admits raw wasm alone, however it hashes — a pin proves the bytes are the ones the entry named, not that they are safe to run as native code, and a registry is not the deployment's build. So the worst a package can name is a hostile guest inside the sandbox, and a `.bin` path is native code the operator vouched for by pinning it. Produce pre-compiled artifacts in your own build pipeline, ship them from immutable or content-addressed storage, and pin every `.bin` path. The same split holds for an embedder's own code: `Runtime::register(id, bytes, grants)` admits raw wasm alone, and a pre-compiled artifact reaches `Runtime::admit` only wrapped in `unsafe { Verified::trusted(bytes) }` — the embedder's word, at the call site, that the bytes came from its own toolchain. Every admitter — the first-use seam, the guest loader, `register` — passes through one body, `Runtime::admit_bytes` under a `Policy` (`Declared`, the entry's pin, format rule, and grants; `CallerNamed`, the call's pin, raw wasm alone, and the grants the call names), so no two paths can drift in what they check.

Two further consequences of dynamic loading:

- **Artifacts are read when they load** — at startup for embedded bytes and `run <component>`, at first use for a manifest file's `source.path` or `source.package`, at every `load` for a path a requester names. A guest path can be substituted between manifest construction and that read; prefer immutable or content-addressed artifact locations, especially for `.bin`, and pin a `digest` on any entry whose file could change under you.
- **Startup cost is unbounded by the runtime.** Nothing caps the manifest's guest count or artifact sizes; compilation cost at startup is bounded only by what the manifest names — another reason the manifest is an operator-privilege input.

Note also that guest-to-guest dispatch is a deployment-level grant with no per-guest ACL on its targets: every interface outside the runtime's own `wasi:` and `omnia:` namespaces is relayed to whichever guest exports it, the linker is shared, and any guest importing such an interface may call it. A guest under grants imports a linked interface only when its `interfaces` list names the package, but which guest answers is bounded by which guests the deployment runs — the operator's choice of components — not by a declared list. The namespace predicate is closed: a native host linked under any namespace other than `wasi:` or `omnia:` collides with the relay polyfill at boot rather than serving its interface, so a third-party host lives under `omnia:` or the predicate grows.

## Guest-requested plugin loading (`omnia:plugins/loader`)

A runtime built with omnia's `loader` feature links the loader capability at assembly, letting a guest whose world imports it request that the host admit another component at run time. The design keeps every trust decision host-side:

- **A source in, a handle out — never bytes.** The interface carries a `source`, with an optional digest pin, in and a typed handle out; component bytes structurally cannot cross it in either direction, and the requester gains no lifecycle authority (no deregister, no mutation of mounts, hosts, or routes). A source is one of three: `declared(name)` names a guest the deployment's `[[guest]]` list declares; `path(file)` names a component file beneath one of the deployment's mounts; `release(reference, registry?, pin?)` names an exact `namespace:name@version`. Each resolves inside the deployment's grant, and the host acquires, verifies, validates, compiles, and publishes the bytes it finds there.
- **The deployment's grant bounds every arm.** `Deployment::assemble` fixes the grant at install: the deployment's guest list as the names a `declared` load may name, the runtime's mounts as the roots a `path` may lie beneath, and the package store and `registries` routing a `release` is acquired through. A `declared` name or a `path` selects within those tables and can widen neither; a `release` load may also name a `registry`, which the deployment's routing outranks. A `declared` name outside the list is `not-found`; one not yet loaded goes through the runtime's own first-use seam, exactly as a route or a link call would load it; one already active (loaded at boot, or used earlier) is attested without fetching anything. It carries no digest of its own — the entry carries the pin. A declared name is bound by its entry alone: a `path` or `release` that would register under it is `denied` before anything is read, whether or not the guest has loaded, so nothing a caller names can seat other bytes under the name for the declared load to attest — the rule is the runtime's admission body's, so an embedder's `Runtime::register` under a declared name is refused the same way. A `path` is read through the mount it lies beneath (`.` for a bare relative path, the mount's name as its prefix otherwise), with cap-std refusing any escape as `denied`, and registers as its file stem. A `release` is acquired by the `RegistryClient` over the deployment's package store (the macro's `plugins: { store: .. }`, a manifest's `[plugins] store`): a release the store holds is served from it, with no network and no registry consulted; one it lacks is fetched by the routing the deployment's `registries` declares (the macro's `plugins: { registries: include_str!("wasm-pkg.toml") }`, a manifest's `[plugins] registries.path`) — package override, then namespace, then `default_registry` — verified against the registry's digest, and written to the store once, never over a file already there. Or by the custom `RegistrySource` an embedder selected with `Deployment::registry_source` before assembly. Either way it registers as its reference without the version. A release the deployment routes is served by its route, and a `registry` naming another is `denied` rather than followed; a release whose namespace the deployment routes nowhere, under no `default_registry`, is fetched from the `registry` the load names, and is `not-found` when it names none, the failure naming the store file that would have served it. Registry egress is therefore bounded by the deployment's routing only as far as that routing reaches: a deployment that admits a loader-importing component it does not author closes the arm itself, with a `default_registry` or by routing every namespace, so that every package a load can name is served by a registry the deployment chose.
- **What a guest can write, it cannot run.** A component loads from a read-only mount alone: a `path` beneath a writable mount is `denied` before the file is read, so a guest with a writable mount cannot plant a component and have the host compile it. Nor can it plant native code where a first use would deserialize it: a manifest file's `source.path` is read when the guest is first used, so a pre-compiled artifact found there is admitted only when the entry pins the bytes, and a guest that overwrites the file gets its planted artifact refused as unpinned, with `invalid` (raw wasm it plants is compiled inside the sandbox, as any raw wasm is). The loader also refuses at assembly a writable mount that shares or nests a read-only mount's directory, since a file written through the one view would load through the other; directories are told apart by identity, not path, so a bind mount or firmlink of a read-only mount's directory is refused as that directory. Two read-only mounts, or two writable ones, may nest freely. The package store is a code root too: a store beneath a writable mount — present, or a directory a guest could create through the mount — is refused at startup by the same identity check, so what a run can write is never where a package is read from. State a guest must write lives under its own mount; code lives under a read-only one, or in the store.
- **Requester-named origins admit raw wasm alone.** A `path` or `release` the requester names is not one of the deployment's `Source`s and never becomes one: its bytes are admitted under `Policy::CallerNamed`, which checks them against the load's pin and then through `Verified::wasm`, refusing a pre-compiled artifact however it hashes as `invalid`, so the worst such a load can name is a hostile guest inside the sandbox. A pre-compiled release a registry serves is refused by the client before it is written, so the store never holds one a later start would read; a pre-compiled file copied into the store by hand is refused at admission the same way. A pre-compiled component is admitted only where the deployment itself vouches for it — embedded bytes, `run <component>`, or a `source.path` the operator pinned.
- **Digest pins are verified before wasmtime sees the bytes.** A `[[guest]]` entry may pin the `digest` its bytes must hash to (`sha256:<hex>`), and a `path` or `release` source may carry one as its `pin`; either is checked against the acquired bytes *before* any wasmtime validation, at boot or at first use alike. A declared entry's bytes that miss its pin are `invalid`. An unpinned load reports the resolved digest on the returned handle (trust-on-first-use), so it can be committed as a pin. A requester-named load whose bytes miss its `pin`, or whose name is already active under other bytes — a path whose stem is a loaded guest, a racing load of changed bytes — loads nothing and answers a `conflict` naming the digest found and the guest active under the name, rather than re-binding it; the same bytes attest.
- **Loaded plugins load as every guest does.** A `declared` load is one more first use of the runtime's guest seam (`Runtime::guest`): the entry's `Source` is read, verified against its pin and the format rule, and admitted exactly as a route or a link call would admit it, under the entry's own grants. A requester-named load reads its path through the mount or fetches its release through the runtime's acquisition (`Runtime::acquire`), then takes the same admission body (`Runtime::admit_bytes`) under the caller-named policy, with no grants (see [the loader's limit](#what-omnia-does-not-protect-against)).
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
| Response streaming | `HTTP_STREAM_TIMEOUT_MS` | off (an HTTP response body streams on after its head, which `GUEST_TIMEOUT_MS` stops at) |
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
- Per guest: a guest without grants sees every mount, and a guest under grants sees only the mounts its `mounts` list names. The mount set is the union of what the deployment's guests legitimately need, kept minimal, and each guest's grants name its share.

## Model completions: lending, not granting ambient access

The `omnia:model` design extends capability thinking to LLM backends, which are effectively untrusted executors:

- The backend gets **no ambient access**. It can touch a filesystem tree only if the guest lends one through `grants.workspace` — and that lend is a typed `wasi:filesystem` descriptor borrow from the guest's own preopen table, not a path string or integer handle a guest could forge. The host resolves it back to an authorized mount by identity. What a backend's worker reaches beyond that lend — the hosts its commands connect to, the toolchain directories it reads, the variables it carries — is the lend's `worker` grant, spelled by the guest per completion and held to its shape by the host, never a deployment-wide setting: a target that knows it runs `cargo` names `~/.rustup` and the crates registry, and nothing else is opened.
- Host-injected tools (`read`, `list`, `write`) are **served and bounded by the host**: the names are reserved, and the backend can only execute them through the host's `ToolHost`, which requires the workspace grant and enforces read/listing bounds (genai advertises `read`/`list` when a workspace is lent; `write` stays unadvertised). Guest-declared function tools are answered by the guest itself over the completion session's streams; the backend forwards each call through the host, which enforces the declared-name allowlist, size cap, and timeout. Guests cannot impersonate the injected tools (reserved names are rejected).
- The **host does not validate the answer**. `format` only steers the provider, so the answer is model output the guest must not trust until it judges it: through `check`, or `Question<T>`, which accepts only a candidate that deserializes as the guest's type and passes its closure.
- Session limits bound runaway tool loops: a call budget and per-call timeout (`exhausted`) and a per-result size cap (`invalid`); the cursor backend additionally cancels its bridge-managed run (`CancelRun`) on timeout or when a session tool call fails hard. Its tool-callback channel is loopback-only and bearer-authenticated, and callbacks route through the same host-enforced `ToolHost::call_tool` path as genai's session tools.

The net effect: a prompt-injected or misbehaving model session is confined to the lent workspace and the granted tools, exactly as a guest is confined to its linked interfaces.

## What Omnia does not protect against

Honest limits, so you can layer the right controls on top:

- **Grants are opt-in.** A guest without a grants table reaches everything the deployment has, and no setting makes a table mandatory. Startup logs each such guest, at `warn` once any guest declares grants, so an operator sees what is unconstrained.
- **The loader is not held to grants.** A guest whose `interfaces` reach `omnia:plugins` loads a `path` from beneath any read-only mount of the deployment, whatever its `mounts` name. A guest it admits by path or release runs with no grants, reaching everything the deployment has. Keep `omnia:plugins` out of the `interfaces` of a guest whose grants matter.
- **Outbound HTTP is coarse.** If `WasiHttp` is linked, the guest can request any URL the host can reach — there is no per-guest URL allow-list today. `interfaces` decides whether a guest has `wasi:http` at all, serving and calling alike. Network egress policy belongs at the infrastructure layer (network policies, egress proxies).
- **The store is the operator's.** A release in the package store is served whoever wrote it and whatever registry a load names, and is never replaced while its file stands: the store is where the operator stages what a deployment may load — `wkg get` into it, or a plain copy — and removing a file is the one refresh. The runtime writes it on the deployment's word alone: a release the `registries` routing fetched is stored, one a load fetched from the `registry` it named is not, so a loading guest cannot plant what a later acquisition of the reference is served. Keep it under a directory no guest mount can write, which the runtime enforces, and no other user can.
- **An unrouted registry namespace is the caller's to route.** A `release` load whose reference the deployment's store lacks and `registries` routes nowhere — no package override, no namespace mapping, no `default_registry` — is fetched from the `registry` the load names: any `host[:port]`, dialled by the host's `RegistryClient` rather than through `WasiHttp` (so a deployment that links no HTTP still has this one egress path), with whatever credentials wasm-pkg resolves for that host (the deployment's `[registry."<host>"]` settings, else the Docker credential store). What it fetches is admitted for that load and never stored. The loader's reach is the deployment's routing plus, for the namespaces it leaves unrouted, any host a loader-importing guest names; where such a guest is not the operator's own, set a `default_registry` or route every namespace.
- **Backend credentials are host-side.** Guests never see connection strings, but any guest with the interface linked can use the backend's full capability (e.g. every bucket the Redis credential can reach). Scope service credentials to what the deployment needs.
- **Within one interface, granularity is the backend's.** No grant lists the buckets, topics, or config keys one guest may name. `omnia:keyvalue` does not partition buckets per guest: a deployment declares its buckets once, and every guest sharing the backend can open each of them. `interfaces` decides whether a guest has the package at all.
- **Writable mounts are real writes.** A writable workspace lent to a model backend can be modified by the model. Review flows should mount read-only and route writes through validated tools.
- **Mount overlap is judged along each mount's own ancestry.** The loader identifies a mount's directory and every directory above it by `(device, inode)`, which catches a second mount point of a code root or of a directory above it. A writable mount that is a second view of a *subdirectory* beneath a read-only mount — a bind mount of `code/out` at `/scratch`, say — presents as an unrelated directory and is not refused; keep such views out of code roots.
- **Denial of service via legitimate traffic** is bounded per invocation, but request admission (rate limiting, auth) is upstream of the runtime.
- **Side channels** (timing, cache) are out of scope, as for most wasm runtimes.

## Defence-in-depth checklist

- [ ] Treat manifests and every artifact they name — pre-compiled ones above all — as trusted operator inputs; never build either from untrusted data
- [ ] Take only raw `.wasm` from less-trusted sources, and run it with minimal hosts and read-only mounts; declare such a guest as a `source.package`, or a `source.path` you leave unpinned, so the runtime itself refuses native code from it; never expect a package or a caller-named load to run `omnia compile` output; feed an embedder's `Runtime::register` raw wasm alone, and reach for `Verified::trusted` only for artifacts your own build produced
- [ ] Pin every `.bin` a manifest's `source.path` names — the pin is what admits it — and every other `[[guest]]` entry by `digest` wherever the artifact is known ahead of time; pin every `path` or `release` source whose bytes are known; treat an unpinned load's reported digest as the pin to commit
- [ ] Mount code read-only and state under a mount of its own; the loader refuses a component beneath a writable mount, and a writable mount that overlaps a read-only one, so the two never share a directory
- [ ] Name a package store (`plugins.store`) outside every writable mount — the runtime refuses one beneath — and treat its contents as code: a release there loads on every start until it is removed
- [ ] Give the loader import only to worlds that genuinely request loads; keep loadable-plugin worlds free of it
- [ ] Set a `default_registry`, or route every namespace, in any deployment whose loader-importing components are not the operator's own; a namespace routed nowhere is fetched from whichever `registry` the load names
- [ ] Link only the interfaces each deployment's guests need
- [ ] Declare `[guest.grants]` for each guest you do not author, naming its packages one by one rather than by prefix, and keep `omnia:plugins` out of its `interfaces`; the startup log names each guest that reaches everything
- [ ] Mount the minimum directory set, read-only unless writes are required
- [ ] Inherit only the host variables guests read (`[env] inherit`), and serve through `omnia:config` only the ones `CONFIG_KEYS` names; `*` in either hands every guest the whole process environment, credentials included
- [ ] Keep resource ceilings meaningful for the workload (don't blanket-raise timeouts and memory)
- [ ] Scope backend service credentials narrowly; prefer per-deployment credentials
- [ ] For model workloads, prefer read-only workspaces; treat `writable` lends as privileged
- [ ] Run the host container as non-root with a minimal image (see [Deploying Omnia](guides/deployment.md#container-images))
