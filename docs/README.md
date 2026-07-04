# Omnia Developer Guide — Local Development

This directory contains the [mdBook](https://rust-lang.github.io/mdBook/) 0.5 source for the Omnia Developer Guide.

## Prerequisites

Install the mdBook 0.5 toolchain locally:

```bash
cargo install --locked mdbook
cargo install --locked mdbook-linkcheck2
```

## Serve (live reload)

```bash
mdbook serve docs    # from the repo root
```

Opens at [http://localhost:3000](http://localhost:3000) by default and live-reloads on chapter or theme changes.

## Build

```bash
mdbook build docs   # from the repo root, runs HTML + linkcheck2
```

Output lands in `docs/book/html/`. Linkcheck2 validates every internal link and fails the build on the first broken reference — see [`book.toml`](book.toml) `[output.linkcheck2]`.

## Custom theme

- Forked mdBook theme: [`theme/`](theme/) — re-vendor from stock on mdBook upgrades.
- Project-owned chrome overrides: [`theme/css/chrome.css`](theme/css/chrome.css), [`theme/head.hbs`](theme/head.hbs).
- Cross-cutting component CSS: [`assets/theme/omnia-docs.css`](assets/theme/omnia-docs.css).

### Re-vendoring the theme after an mdBook upgrade

1. In a temp directory: `mdbook init --theme tmp-book` using the target mdBook version.
2. Copy `tmp-book/theme/*` into [`docs/theme/`](theme/), replacing stock files.
3. Re-apply project-owned customisations:
   - Augentic brand + breadcrumb in [`theme/index.hbs`](theme/index.hbs) (`menu-title`, `spec-footer`).
   - Banner block at the bottom of [`theme/css/chrome.css`](theme/css/chrome.css).
   - [`theme/head.hbs`](theme/head.hbs) and system-font override in [`theme/fonts/fonts.css`](theme/fonts/fonts.css).
4. Run `mdbook build docs` and spot-check light + navy themes on [`index.md`](index.md) and [`Architecture.md`](Architecture.md).
