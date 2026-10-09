# Omnia WASI VCS

This crate provides the version-control interface for the Omnia runtime: the `omnia:vcs` boundary a guest reaches repositories and working copies through, in the nouns git and jj share.

## Interface

Implements the `omnia:vcs@0.1.0` WIT package ([`wit/vcs.wit`](wit/vcs.wit)), three interfaces a backend exports what it has of:

| Interface | Operations | Purpose |
| --------- | ---------- | ------- |
| `store` | `resolve`, `descends`, `head`, `commit`, `merge`, `log` | The commit graph: sealed snapshots, whether one commit is in another's history, the merge under a policy, and the first-parent chain over a base |
| `workspace` | `init`, `add`, `remove`, `pending` | Working copies laid beneath a mount, and what each holds past its head |
| `transport` | `clone`, `fetch`, `label`, `labelled`, `push` | Remotes and labels: a label is written and read back in one namespace, and a push never forces |

Every operation names its repository or working copy as a `location`: a borrowed mount-root descriptor plus a plain relative subpath, the `workspace-grant` idiom of `omnia:model`. The host resolves it against the deployment's mount registry before the backend runs, so a backend works only beneath a mount the deployment authorised, and a mutation (`init`, `add`, `remove`, `commit`, `merge`, `clone`, `fetch`, `label`, `push`) beneath a read-only mount is refused as `error.other` before any backend runs. The backend receives a `Place`, an open directory handle beneath the mount that the host created for `init`, `add`, and `clone` where nothing stood and holds for as long as the backend runs, so a guest rearranging its tree meanwhile cannot redirect the operation; it never sees a descriptor, and the guest sees locations and never a path. A subpath that does not exist is `not-a-repository` for every other operation, before any backend runs.

A typed `error` — `not-a-repository`, `exists`, `not-found`, `pending`, `access`, `diverged`, `other` — crosses the boundary as the backend returned it; any other backend failure reaches the guest as `other`.

## Backend

There is no default backend: nothing in memory stands for a repository. A deployment that names no `WasiVcs` row links no `omnia:vcs` import, and a guest that imports it fails to instantiate there.

- **Production**: [`omnia-git`](https://github.com/augentic/omnia-backends/tree/main/crates/git) — the operator's `git` binary, one process per operation (see the [Production Backends guide](https://github.com/augentic/omnia/blob/main/docs/guides/production-backends.md)).
- **Tests**: a scripted `WasiVcsCtx` of the suite's own, set with `omnia_test::host::Backends::vcs`; the bundle holds `NoVcs` in the slot until then, so a deployment linking `WasiVcs` over the defaults fails to link.

## Usage

Add this crate to your `Cargo.toml` and name a backend in your runtime configuration:

```rust,ignore
use omnia_git::Client as Git;
use omnia_wasi_vcs::WasiVcs;

omnia::runtime!({
    hosts: {
        WasiVcs: Git,
    }
});
```

A guest reaches the interface through `omnia_sdk::Vcs`, naming every location as a deployment-local path (`"."` the project mount, `"./.cache/repo"` beneath it), or through the raw `omnia_wasi_vcs::{store, workspace, transport}` bindings.

## License

MIT OR Apache-2.0
