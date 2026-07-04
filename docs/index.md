<div class="hero">
<div class="eyebrow">Omnia Developer Guide</div>
<h1 class="hero-title">Sandboxed guests, production hosts</h1>

Omnia is a lightweight runtime for WebAssembly (WASI) components built on [wasmtime](https://github.com/bytecodealliance/wasmtime). Application code compiles to a sandboxed **guest**; a native **host** provides HTTP, storage, messaging, SQL, model completions, and more through standard WASI interfaces. The same guest runs unchanged against in-memory defaults on a laptop or production services in deployment.

<div class="meta-row">

<span class="meta-chip"><strong>Runtime</strong> wasmtime + WASI P3</span>

<span class="meta-chip"><strong>Defaults</strong> in-memory, no external services</span>

<span class="meta-chip"><strong>Deploy</strong> swap in Redis, Kafka, Azure, Postgres</span>

</div>

</div>

<div class="proof-strip">
  <div class="proof-item">
    <div class="proof-kicker">Start</div>
    <div class="proof-value">10 min</div>
    <div class="proof-copy">Build and run your first HTTP guest with in-memory defaults — no databases or credentials.</div>
  </div>
  <div class="proof-item">
    <div class="proof-kicker">Model</div>
    <div class="proof-value">Guest + host</div>
    <div class="proof-copy">Guests stay sandboxed; the host mediates every capability through WASI interfaces.</div>
  </div>
  <div class="proof-item">
    <div class="proof-kicker">Scale</div>
    <div class="proof-value">Same WASM</div>
    <div class="proof-copy">Swap in-memory defaults for production backends without recompiling the guest.</div>
  </div>
</div>

## Start here

<div class="card-grid">
  <a class="card" href="getting-started.html">
    <div class="card-head">
      <div class="card-title">Getting Started</div>
      <span class="card-time">~10 min</span>
    </div>
    <div class="card-body"><p>Build and run your first guest in about ten minutes.</p></div>
  </a>
  <a class="card" href="guides/writing-guests.html">
    <div class="card-head">
      <div class="card-title">Writing Guests</div>
    </div>
    <div class="card-body"><p>HTTP handlers, WASI capabilities, tracing, and command-mode guests.</p></div>
  </a>
  <a class="card" href="guides/composing-a-runtime.html">
    <div class="card-head">
      <div class="card-title">Composing a Runtime</div>
    </div>
    <div class="card-body"><p>The <code>runtime!</code> macro, choosing hosts and backends, server vs command mode.</p></div>
  </a>
</div>

## How the guide is organised

<div class="rhythm">
  <div class="rhythm-step">
    <div class="rhythm-num">01</div>
    <div class="rhythm-label">How-to</div>
    <div class="rhythm-title">Task-focused guides</div>
    <p>SQL, document store, messaging, model completions, deployment, and performance tuning.</p>
  </div>
  <div class="rhythm-step">
    <div class="rhythm-num">02</div>
    <div class="rhythm-label">Explain</div>
    <div class="rhythm-title">Architecture and security</div>
    <p>How the runtime is put together, what the sandbox guarantees, and shared terminology.</p>
  </div>
  <div class="rhythm-step">
    <div class="rhythm-num">03</div>
    <div class="rhythm-label">Reference</div>
    <div class="rhythm-title">Interfaces and configuration</div>
    <p>WASI interface matrix, model types, CLI flags, and environment variables.</p>
  </div>
</div>

<div class="see-also">

**See also**

- [Architecture](Architecture.md) — three-layer design, guest registry, instance pooling
- [WASI Interfaces](reference/wasi-interfaces.md) — every interface crate and its backends
- [Troubleshooting](troubleshooting.md) — common build, startup, and runtime failures

</div>
