## 0.38.0

Unreleased

### Added

- `omnia_sdk::api::{Handlers, Router}`: a module registers its HTTP routes
  (`route`), messaging topics (`topic`, `topic_with`), and CLI subcommands
  (`command`, `command_with`, behind the `command` feature) on one
  `Handlers<P>`, `From<[..; N]>` (or `merge`) merges them, and
  `with_state(client)` closes the whole into the one `Router<P>` that
  serves requests, handles deliveries, and `run`s argv. `command::export!`
  binds a command router fn (with `WasiProvider` or a custom provider)
  alongside a zero-argument `async fn` entry, stamping the guest crate's
  `CARGO_PKG_VERSION` and `CARGO_PKG_DESCRIPTION` as the program's
  `--version` and `--help` defaults.
- `omnia_sdk::api::Render`: the text form of a handler output, which
  `Format::Text` encodes (`()` and `String` implement it); `Failure`
  renders its envelope through it.

### Changed

- `command_with`'s decoder returns `DecodeError`, as `topic_with`'s does,
  and `Command::call(handler, input)` takes the decoded input.
  `Format::encode(body)` renders through `Render`, and `Failure` renders
  its envelope through `Render` in place of `Failure::text`.

### Removed

- `omnia_sdk::api::command::{Commands, Router}`: subcommands are
  `Handlers::command(name, handler)` beside a module's paths and topics,
  the output's `Render` impl replaces the per-verb render fn, and the
  router's `run`/`run_with` replace `handle`/`handle_with`.
  `Commands::map` has no successor.

---

Release notes for previous releases can be found on the respective release branches of the repository.

<!-- ARCHIVE_START -->
* [0.37.x](https://github.com/augentic/omnia/blob/release-0.37.0/RELEASES.md)
* [0.36.x](https://github.com/augentic/omnia/blob/release-0.36.0/RELEASES.md)
* [0.35.x](https://github.com/augentic/omnia/blob/release-0.35.0/RELEASES.md)
* [0.34.x](https://github.com/augentic/omnia/blob/release-0.34.0/RELEASES.md)
* [0.33.x](https://github.com/augentic/omnia/blob/release-0.33.0/RELEASES.md)
* [0.32.x](https://github.com/augentic/omnia/blob/release-0.32.0/RELEASES.md)
* [0.31.x](https://github.com/augentic/omnia/blob/release-0.31.0/RELEASES.md)
* [0.30.x](https://github.com/augentic/omnia/blob/release-0.30.0/RELEASES.md)
* [0.29.x](https://github.com/augentic/omnia/blob/release-0.29.0/RELEASES.md)
* [0.28.x](https://github.com/augentic/omnia/blob/release-0.28.0/RELEASES.md)
* [0.27.x](https://github.com/augentic/omnia/blob/release-0.27.0/RELEASES.md)
* [0.25.x](https://github.com/augentic/omnia/blob/release-0.25.0/RELEASES.md)
* [0.23.x](https://github.com/augentic/omnia/blob/release-0.23.0/RELEASES.md)
* [0.22.x](https://github.com/augentic/omnia/blob/release-0.22.0/RELEASES.md)
* [0.21.x](https://github.com/augentic/omnia/blob/release-0.21.0/RELEASES.md)
* [0.20.x](https://github.com/augentic/omnia/blob/release-0.20.0/RELEASES.md)
* [0.19.x](https://github.com/augentic/omnia/blob/release-0.19.0/RELEASES.md)
* [0.18.x](https://github.com/augentic/omnia/blob/release-0.18.0/RELEASES.md)
* [0.17.x](https://github.com/augentic/omnia/blob/release-0.17.0/RELEASES.md)
* [0.16.x](https://github.com/augentic/omnia/blob/release-0.16.0/RELEASES.md)
* [0.15.x](https://github.com/augentic/omnia/blob/release-0.15.0/RELEASES.md)
