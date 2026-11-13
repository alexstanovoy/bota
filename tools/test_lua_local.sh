#!/usr/bin/env bash
# Explicit optional local-source conformance; no sources are fetched or recorded.
set -euo pipefail
: "${BOTA_LUA_LOCAL_PACK:?set BOTA_LUA_LOCAL_PACK to a local source directory}"
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
cargo test -p bota_lua --profile test-release --features native --test local_scripts -- --ignored
