# Widget Service Overview

The Widget Service manages the lifecycle of widgets: creation, assembly, and
shipping. It is intentionally small so it can be used as a reference when
building agent-driven tooling.

## Concepts

- **Widget**: the unit of work. Every widget has a stable `id` and a `state`.
- **Assembly**: the process that moves a widget from `draft` to `assembled`.
- **Manifest**: the shipping record produced once a widget is `assembled`.

## Lifecycle

A widget moves through three states in order: `draft`, `assembled`, `shipped`.
A widget can never move backwards; a mistake is corrected by superseding the
widget with a new one that references the original via `supersedes`.
