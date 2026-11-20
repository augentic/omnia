#!/usr/bin/env bash
# Packages the omnia:* WIT for the wasm-pkg registry, and keeps the packages
# a crate copies into its wit/deps in step with the packages that own them.
#
#   scripts/wasm-pkg.sh sync     rewrite every copy under a crate's wit/deps from
#                           the package of that name
#   scripts/wasm-pkg.sh check    fail if a copy is stale, a published package
#                           holds a plain comment, or a package does not
#                           parse with and without its features, or build
#   scripts/wasm-pkg.sh stage    stage omnia:sdk, the aggregate package bindgen.sh
#                           generates from, under target/omnia-sdk-wit
#   scripts/wasm-pkg.sh build    encode every published package, with metadata,
#                           to target/wit/<namespace>_<name>@<version>.wasm
#   scripts/wasm-pkg.sh verify   fail if a release the registry holds differs
#                           from the package the tree builds
#   scripts/wasm-pkg.sh publish [--overwrite PACKAGE,..]
#                           verify, then publish every release the registry
#                           lacks; --overwrite republishes the named
#                           packages' differing releases instead of failing
#
# Tool pins (bump with mise.toml and the CI install-action):
#   jq 1.8, wasm-tools 1.263.0, wkg 0.16.2
#
# The published packages are every crates/wasi-*/wit and crates/plugin/wit
# package, omnia:types first. The staged omnia:sdk is not one of them. A
# copy's owner is the published package of its name, or the WASI package the
# locked wasip3 ships. Each package is encoded with every `@unstable`
# feature, since the gates travel with it and a consumer enables its own.
# Its description is the first paragraph of its package docs, and the rest
# of its metadata is its crate's. A release is compared as WIT text, so
# metadata alone is never a change.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

config="$root/.wkg-config.toml"
stage="target/omnia-sdk-wit"
out="target/wit"

metadata="$(cargo metadata --format-version 1 --locked)"
wasi="$(jq -r '.packages[] | select(.name == "wasip3") | .manifest_path' <<<"$metadata")"
wasi="$(dirname "$wasi")/wit/deps"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

fail() {
  echo "$*" >&2
  exit 1
}

package_of() {
  awk '/^package [^ ;{]+;/ { sub(/^package /, ""); sub(/;.*/, ""); print; exit }' "$@"
}

# the published package directories, in publish order
sources() {
  echo crates/wasi-types/wit
  for dir in crates/wasi-*/wit crates/plugin/wit; do
    if [ "$dir" != crates/wasi-types/wit ]; then
      echo "$dir"
    fi
  done
}

# --- copies ---

copies() {
  find crates/*/wit/deps -type f -name '*.wit' | sort
}

owner_of() {
  local name="$1" file package
  for file in crates/wasi-*/wit/*.wit crates/plugin/wit/*.wit "$wasi"/*.wit; do
    package="$(package_of "$file")"
    if [ "${package%@*}" = "$name" ]; then
      echo "$file"
      return
    fi
  done
  fail "no published or WASI package owns $name"
}

sync_copies() {
  local copy package owner
  for copy in $(copies); do
    package="$(package_of "$copy")"
    owner="$(owner_of "${package%@*}")"
    if ! cmp -s "$owner" "$copy"; then
      cp "$owner" "$copy"
      echo "synced $copy from $owner"
    fi
  done
}

# --- check ---

check() {
  local status=0 copy package owner dir

  # copies in step with their owners
  for copy in $(copies); do
    package="$(package_of "$copy")"
    owner="$(owner_of "${package%@*}")"
    if ! cmp -s "$owner" "$copy"; then
      echo "$copy differs from $owner: run \`mise run wit-sync\`" >&2
      status=1
    fi
  done

  # no plain comments, which wit-parser publishes as the next item's docs
  for dir in $(sources); do
    if grep -nHE '^[[:space:]]*//([^/]|$)' "$dir"/*.wit >&2; then
      echo "a plain \`//\` comment is published as docs: write \`///\` docs, or keep the note out of the WIT" >&2
      status=1
    fi
  done

  # every package parses as a consumer with no features and one with all
  stage
  for dir in crates/*/wit "$stage"; do
    wasm-tools component wit "$dir" >/dev/null || status=1
    wasm-tools component wit --all-features "$dir" >/dev/null || status=1
  done

  if [ "$status" != 0 ]; then
    exit 1
  fi
  build
}

# --- stage ---

stage() {
  local version file package name includes messaging websocket cli
  version="$(jq -r '.packages[] | select(.name == "omnia-sdk") | .version' <<<"$metadata")"
  messaging="$(package_of crates/wasi-messaging/wit/*.wit)"
  websocket="$(package_of crates/wasi-websocket/wit/*.wit)"
  cli="$(package_of "$wasi/cli.wit")"

  rm -rf "$stage"
  mkdir -p "$stage/deps"
  cp "$wasi"/*.wit "$stage/deps/"

  includes=""
  for file in crates/wasi-*/wit/*.wit crates/plugin/wit/*.wit; do
    package="$(package_of "$file")"
    name="${package%@*}"
    mkdir -p "$stage/deps/${name/:/-}"
    cp "$file" "$stage/deps/${name/:/-}/"
    if grep -q '^world imports ' "$file"; then
      includes="$includes  include $name/imports@${package#*@};"$'\n'
    fi
  done

  cat >"$stage/sdk.wit" <<EOF
package omnia:sdk@$version;

world imports {
$includes}

world messaging {
  export ${messaging%@*}/incoming-handler@${messaging#*@};
}

world websocket {
  export ${websocket%@*}/incoming-handler@${websocket#*@};
}

world command {
  include imports;
  include ${cli%@*}/command@${cli#*@};
}
EOF
}

# --- build ---

build() {
  local revision dir
  revision="$(git rev-parse HEAD 2>/dev/null || true)"
  rm -rf "$out"
  mkdir -p "$out"
  : >"$out/index"
  for dir in $(sources); do
    encode "$dir" "$revision"
  done
}

encode() {
  local dir="$1" revision="$2" package crate file description
  local authors licenses source homepage
  local -a fields=()
  package="$(package_of "$dir"/*.wit)"
  if [ -z "$package" ]; then
    fail "$dir declares no package"
  fi
  crate="${dir%/wit}"
  file="$out/${package/:/_}.wasm"

  description="$(wasm-tools component wit --json --all-features "$dir" | jq -r --arg package "$package" '
    .packages[] | select(.name == $package) | .docs.contents // ""
    | split("\n\n")[0] | gsub("\\s*\n\\s*"; " ") | gsub("`"; "")')"
  if [ -z "$description" ]; then
    fail "$package has no package docs: add a \`///\` summary above its \`package\` line"
  fi
  IFS=$'\x1f' read -r authors licenses source homepage <<<"$(jq -r --arg manifest "/$crate/Cargo.toml" '
    .packages[] | select(.manifest_path | endswith($manifest))
    | [(.authors | join(", ")), .license, .repository, .homepage] | map(. // "") | join("\u001f")' <<<"$metadata")"

  [ -z "$authors" ] || fields+=(--authors "$authors")
  [ -z "$licenses" ] || fields+=(--licenses "$licenses")
  [ -z "$source" ] || fields+=(--source "$source")
  [ -z "$homepage" ] || fields+=(--homepage "$homepage")
  [ -z "$revision" ] || fields+=(--revision "$revision")

  wasm-tools component wit --wasm --all-features "$dir" -o "$work/package.wasm"
  wasm-tools metadata add "$work/package.wasm" -o "$file" \
    --description "$description" --version "${package#*@}" ${fields[@]+"${fields[@]}"}
  printf '%s\t%s\n' "$package" "$file" >>"$out/index"
  echo "built $file"
}

# --- registry ---

wkg_quiet() {
  RUST_LOG="${RUST_LOG:-error}" wkg "$@" --config "$config" --cache "$work/cache" </dev/null
}

# whether the registry holds $1 as `absent`, the `same` WIT as $2, or WIT
# that `differs`, which goes to stderr as a diff
release() {
  local package="$1" file="$2" log="$work/get.log"
  if wkg_quiet get "$package" --format wasm --overwrite -o "$work/release.wasm" >"$log" 2>&1; then
    wasm-tools component wit "$work/release.wasm" >"$work/release.wit"
    wasm-tools component wit "$file" >"$work/tree.wit"
    if diff -u --label "$package (published)" --label "$package (tree)" \
      "$work/release.wit" "$work/tree.wit" >&2; then
      echo same
    else
      echo differs
    fi
  elif grep -qiE 'manifest unknown|name unknown|package not found|version not found' "$log"; then
    echo absent
  else
    cat "$log" >&2
    fail "$package: no release could be read through the registry .wkg-config.toml routes it to"
  fi
}

verify() {
  local status=0 package file state
  build
  while IFS=$'\t' read -r package file; do
    state="$(release "$package" "$file")"
    echo "$package: $state"
    if [ "$state" = differs ]; then
      status=1
    fi
  done <"$out/index"
  if [ "$status" != 0 ]; then
    fail "a published release differs from the tree: give each package that differs a new version"
  fi
}

publish() {
  local overwrite="" status=0 package file state entry
  local -a entries=()
  local plan="$work/plan"
  while [ $# -gt 0 ]; do
    case "$1" in
      --overwrite)
        overwrite="${2:?--overwrite takes a comma-separated list of packages}"
        shift 2
        ;;
      --overwrite=*)
        overwrite="${1#*=}"
        shift
        ;;
      *) fail "unknown argument: $1" ;;
    esac
  done
  if [ -n "$overwrite" ]; then
    IFS=, read -r -a entries <<<"$overwrite"
  fi

  build
  for entry in ${entries[@]+"${entries[@]}"}; do
    if ! awk -F'\t' -v name="${entry%@*}" '{ sub(/@.*/, "", $1) } $1 == name { found = 1 }
      END { exit !found }' "$out/index"; then
      fail "--overwrite names ${entry%@*}, which the tree does not publish"
    fi
  done

  # every release decided before any is published
  : >"$plan"
  while IFS=$'\t' read -r package file; do
    state="$(release "$package" "$file")"
    if [ "$state" = differs ]; then
      for entry in ${entries[@]+"${entries[@]}"}; do
        if [ "${entry%@*}" = "${package%@*}" ]; then
          state=overwrite
        fi
      done
    fi
    echo "$package: $state"
    if [ "$state" = differs ]; then
      echo "$package differs from its release: give it a new version" >&2
      status=1
    fi
    printf '%s\t%s\t%s\n' "$package" "$file" "$state" >>"$plan"
  done <"$out/index"
  if [ "$status" != 0 ]; then
    fail "nothing published"
  fi

  # the releases the registry lacks, omnia:types first
  while IFS=$'\t' read -r package file state; do
    case "$state" in
      absent) wkg_quiet publish "$file" ;;
      overwrite) wkg_quiet publish --skip-semver-check "$file" ;;
    esac
  done <"$plan"
}

case "${1:-}" in
  sync) sync_copies ;;
  check) check ;;
  stage) stage ;;
  build) build ;;
  verify) verify ;;
  publish)
    shift
    publish "$@"
    ;;
  *) fail "usage: scripts/wasm-pkg.sh sync|check|stage|build|verify|publish [--overwrite PACKAGE,..]" ;;
esac
