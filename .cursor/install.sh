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

# the cargo tools CI installs, and sccache
command -v cargo-binstall >/dev/null ||
  curl -fsSL https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash
cargo binstall --no-confirm cargo-nextest cargo-hack cargo-deny cargo-vet@0.10.2 sccache

# compile through sccache, falling back to plain rustc if its server fails
config="${CARGO_HOME:-$HOME/.cargo}/config.toml"
if ! grep -qs '^\[build\]' "$config"; then
  cat >>"$config" <<EOF
[build]
rustc-wrapper = "$(command -v sccache)"

[env]
SCCACHE_CACHE_SIZE = "5G"
SCCACHE_IGNORE_SERVER_IO_ERROR = "1"
EOF
fi

# mise, trusted to run this repo's tasks, with the shared tasks fetched
command -v mise >/dev/null || curl -fsSL https://mise.run | sh
mise trust
mise tasks ls >/dev/null
mise install jq@1.8

# a warm target/ for the commands AGENTS.md verifies with
cargo fetch --locked
cargo nextest run --locked --workspace --all-features --no-run
cargo clippy --locked --workspace --all-targets --all-features
