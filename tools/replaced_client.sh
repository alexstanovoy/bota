#!/usr/bin/env bash
# Build the client immediately preceding the definition-name removal, without admitting its wire.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
base=0bb400bad81a3d614f02a31417b31dbc4eac4b66
cache=${CARGO_TARGET_DIR:-$root/target}/replaced_client
mkdir -p -- "$cache"
cache=$(cd -- "$cache" && pwd)
work=$(mktemp -d "$cache/source.XXXXXX")
trap 'rm -rf -- "$work"' EXIT
mkdir "$work/bota"
git -C "$root" archive "$base" | tar -x -C "$work/bota"
ln -s -- "$root/../radovid" "$work/radovid"
mkdir -p "$work/bota/crates/bota_client_core/examples"
cp "$root/crates/bota_server/tests/wire/replaced_peer.rs" "$work/bota/crates/bota_client_core/examples/replaced_peer.rs"
cargo build --locked --release --manifest-path "$work/bota/Cargo.toml" \
    --target-dir "$cache/build" -p bota_client_core --example replaced_peer
cp "$cache/build/release/examples/replaced_peer" "$cache/peer"
