#!/usr/bin/env bash
# Regenerates the omnia-sdk guest bindings from the WIT the host crates own.
#
#   scripts/bindgen.sh                  regenerate crates/sdk/src/bindings
#   scripts/bindgen.sh --check [GUEST]  fail if a regeneration differs from
#                                       the committed bindings, then check
#                                       that GUEST (a command component)
#                                       targets the staged world; with no
#                                       GUEST, build config_seeded for
#                                       wasm32-wasip3 and use the default path
#
# Tool pins (bump with workspace wit-bindgen and CI install-action):
#   jq 1.8, wit-bindgen-cli 0.63.0, wasm-tools 1.263.0
#
# `scripts/wit.sh stage` stages every `crates/wasi-*/wit` and
# `crates/plugin/wit` package under `target/omnia-sdk-wit`, beside the WASI
# packages the pinned `wasip3` ships, with one aggregate package, `omnia:sdk`,
# that includes each package's `imports` world. The output follows
# `wasi-rs`'s `ci/regenerate.sh`: one `imports.rs` for every import, and one
# file per export world whose import types point back at it. Every
# `@unstable` feature is enabled, since every host links the gated
# interfaces.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

check=false
guest=""
if [ "${1:-}" = "--check" ]; then
  check=true
  guest="${2:-}"
fi

metadata="$(cargo metadata --format-version 1 --locked)"
version="$(jq -r '.packages[] | select(.name == "omnia-sdk") | .version' <<<"$metadata")"

# --- stage ---

stage="target/omnia-sdk-wit"
scripts/wit.sh stage

version_of() {
  awk '/^package [^ ;{]+;/ { sub(/^package [^@]*@/, ""); sub(/;.*/, ""); print; exit }' "$@"
}

# --- generate ---

out="crates/sdk/src/bindings"
if $check; then
  out="$(mktemp -d)"
  trap 'rm -rf "$out"' EXIT
fi
mkdir -p "$out"

generate() {
  wit-bindgen rust "$stage" --out-dir "$out" --format --all-features \
    --runtime-path wasip3::wit_bindgen::rt "$@"
}

filesystem="$(version_of "$stage/deps/filesystem.wit")"
clocks="$(version_of "$stage/deps/clocks.wit")"
wasi="wasi:filesystem/types@$filesystem=wasip3::filesystem::types"
wasi="$wasi,wasi:clocks/system-clock@$clocks=wasip3::clocks::system_clock"
wasi="$wasi,wasi:clocks/types@$clocks=wasip3::clocks::types"
generate \
  --world omnia:sdk/imports \
  --generate-all \
  --with "$wasi" \
  --type-section-suffix "rust-omnia-sdk-$version"

types="$(version_of crates/wasi-types/wit/*.wit)"
for world in messaging websocket; do
  with="omnia:types/types@$types=crate::bindings::imports::omnia::types::types"
  with="$with,omnia:$world/types@$(version_of "crates/wasi-$world/wit/"*.wit)"
  with="$with=crate::bindings::imports::omnia::$world::types"
  generate \
    --world "omnia:sdk/$world" \
    --with "$with" \
    --type-section-suffix "rust-omnia-sdk-$version-$world-world" \
    --default-bindings-module '$crate' \
    --pub-export-macro \
    --export-macro-name "_export_$world"
done

# --- check ---

if $check; then
  diff -ru crates/sdk/src/bindings "$out" || {
    echo "crates/sdk/src/bindings is stale: run \`mise run bindgen\`" >&2
    exit 1
  }
  if [ -z "$guest" ]; then
    cargo build --locked -p test-programs --example config_seeded --target wasm32-wasip3
    guest="${CARGO_TARGET_DIR:-$root/target}/wasm32-wasip3/debug/examples/config_seeded.wasm"
  fi
  wasm-tools component targets --all-features "$stage" "$guest" --world omnia:sdk/command
fi
