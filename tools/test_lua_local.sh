#!/usr/bin/env bash
# Explicit optional local-source conformance; no sources are fetched or recorded.
# Needs the luagee checkout beside this repository (crates/bota_lua is a workspace of its own).
set -euo pipefail
: "${BOTA_LUA_LOCAL_PACK:?set BOTA_LUA_LOCAL_PACK to a local source directory}"
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
cargo test --manifest-path crates/bota_lua/Cargo.toml --profile test-release --features native --test local_scripts -- --ignored
