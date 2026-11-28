#!/usr/bin/env bash
# Prepares a Cursor cloud agent Build: the toolchains and tools AGENTS.md
# verifies with, then a warm target/. A Build runs this once on the default
# branch and saves the disk, so every agent starts from its result. It runs
# again over a saved disk, so every step is safe to repeat.
set -euo pipefail

export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
sudo=""
[ "$(id -u)" -eq 0 ] || sudo="sudo"

# native libraries the crates build against
packages=(build-essential cmake curl git jq libcurl4-openssl-dev libssl-dev perl pkg-config protobuf-compiler zlib1g-dev)
if ! dpkg -s "${packages[@]}" >/dev/null 2>&1; then
  $sudo apt-get update -qq
  $sudo apt-get install -y -qq --no-install-recommends "${packages[@]}"
fi

# stable as rust-toolchain.toml names it, and nightly rustfmt
command -v rustup >/dev/null ||
  curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none
rustup toolchain install || rustup show
rustup update --no-self-update stable
rustup toolchain install nightly --profile minimal --component rustfmt

# the cargo tools CI installs, sccache, and mise, at pinned releases
#
# Each archive is checked against the SHA-256 recorded here before it is
# unpacked, and every run reinstalls, so a saved disk keeps no binary this
# script did not verify. Bump a version and its digest together.
[ "$(uname -m)" = x86_64 ] || { echo "install.sh pins x86_64 releases only" >&2; exit 1; }
bin="${CARGO_HOME:-$HOME/.cargo}/bin"
fetch() {
  local url=$1 sha256=$2 member=$3 dest=$4 dir
  dir=$(mktemp -d)
  curl -fsSL "$url" -o "$dir/archive.tar.gz"
  echo "$sha256  $dir/archive.tar.gz" | sha256sum --check --quiet
  tar -xzf "$dir/archive.tar.gz" -C "$dir" "$member"
  install -D -m 755 "$dir/$member" "$dest"
  rm -rf "$dir"
}
fetch https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-0.9.152/cargo-nextest-0.9.152-x86_64-unknown-linux-gnu.tar.gz \
  d039b43c3e0958441b6ad0a39c8884d018b3d2e8fb9928f209074b1100b5e17a \
  cargo-nextest "$bin/cargo-nextest"
fetch https://github.com/taiki-e/cargo-hack/releases/download/v0.6.46/cargo-hack-x86_64-unknown-linux-gnu.tar.gz \
  ff1b9441986e494d32ce46a7c5a25af1cda3a18da2cde306b2f96dd0e7055b28 \
  cargo-hack "$bin/cargo-hack"
fetch https://github.com/EmbarkStudios/cargo-deny/releases/download/0.22.2/cargo-deny-0.22.2-x86_64-unknown-linux-musl.tar.gz \
  9f831cfb5d03a5093c0810091a7bd3b5e4724a1288cbe5718d8ce8cef2590adf \
  cargo-deny-0.22.2-x86_64-unknown-linux-musl/cargo-deny "$bin/cargo-deny"
fetch https://github.com/mozilla/sccache/releases/download/v0.20.1/sccache-v0.20.1-x86_64-unknown-linux-musl.tar.gz \
  c9f6c5a3ae84d9da0956b28c357a365827bda33f3383647fc0b67484c74a5ea9 \
  sccache-v0.20.1-x86_64-unknown-linux-musl/sccache "$bin/sccache"
fetch https://github.com/jdx/mise/releases/download/v2026.11.14/mise-v2026.11.14-linux-x64.tar.gz \
  42a3ff51c24b4315d679a07bbc598e299dda12fea0dfc0d6f90a543495834831 \
  mise/bin/mise "$HOME/.local/bin/mise"

# compile through that sccache, falling back to plain rustc if its server fails
config="${CARGO_HOME:-$HOME/.cargo}/config.toml"
wrapper="rustc-wrapper = \"$bin/sccache\""
if ! grep -qsF "$wrapper" "$config"; then
  if grep -qs '^\[build\]' "$config"; then
    echo "$config has a [build] table without $wrapper" >&2
    exit 1
  fi
  cat >>"$config" <<EOF
[build]
$wrapper

[env]
SCCACHE_CACHE_SIZE = "5G"
SCCACHE_IGNORE_SERVER_IO_ERROR = "1"
EOF
fi

# cargo-vet publishes no binary after 0.10.0, so it builds from its
# crates.io source against its own lockfile
cargo install --locked --force cargo-vet@0.10.2

# mise, trusted to run this repo's tasks, with the shared tasks fetched
mise trust
mise tasks ls >/dev/null
mise install jq@1.8

# a warm target/ for the commands AGENTS.md verifies with
cargo fetch --locked
cargo nextest run --locked --workspace --all-features --no-run
cargo clippy --locked --workspace --all-targets --all-features
