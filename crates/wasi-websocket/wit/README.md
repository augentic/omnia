# WebAssembly Interface Types (WIT) Deps

## Prerequisites

Install `wit-deps` from source (<https://github.com/bytecodealliance/wit-deps>)

## Usage

Add dependencies to `deps.toml`:

```toml
keyvalue = "https://github.com/augentic/wasi-keyvalue/archive/main.tar.gz"
```

Import/update dependencies using `wit-deps update` from the crate root.
