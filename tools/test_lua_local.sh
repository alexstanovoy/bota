#!/usr/bin/env bash
# Explicit optional local-source conformance; no sources are fetched or recorded.
set -euo pipefail
: "${BOTA_LUA_LOCAL_PACK:?set BOTA_LUA_LOCAL_PACK to a local source directory}"
cargo test --profile test-release -p bota_lua --features native --test local_scripts -- --ignored
