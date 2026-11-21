#!/usr/bin/env bash
# A game patch's numbers, moved in with one command: every table tools/import_items generates (items, heroes,
# lane creeps, Roshan, units, neutral creeps and their abilities, neutral items, waves, no-ward zones) is read
# again from a Dota 2 install, and each names the client build it was read from.
#
#   tools/patch_move.sh INSTALL           rewrites the tables from INSTALL (a "dota 2 beta" folder) and lists
#                                         what changed
#   tools/patch_move.sh INSTALL --check   writes nothing; fails when a committed table holds other rows than
#                                         INSTALL gives, or was read from another build
#
# What it does not do: rules that are not tables (crates/bota_core/src/rules.rs and the other typed constants)
# follow the patch notes by hand, and the determinism pins of a rules change are re-recorded once the tables
# and rules are in (DEVELOPMENT.md: the ignored `print_pins` and `print_blind_pins` tests). Run it from a clean
# tree: the list of changes is the tree's diff.
set -euo pipefail
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
[ $# -ge 1 ] && [ $# -le 2 ] || { sed -n '2,/^set -euo/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//' >&2; exit 2; }
install=$1
check=${2:-}
[ -z "$check" ] || [ "$check" = --check ] || { echo "patch_move: the second argument can only be --check" >&2; exit 2; }
[ -f "$install/game/dota/steam.inf" ] || { echo "patch_move: $install is not a \"dota 2 beta\" folder (no game/dota/steam.inf)" >&2; exit 2; }
cd -- "$here"
cargo build --locked --release --quiet --manifest-path tools/import_items/Cargo.toml
BOTA_DOTA_PATH=$install tools/import_items/target/release/import_items ${check:+"$check"}
if [ -z "$check" ]; then
    git diff --stat -- crates/bota_core/src crates/bota_proto/registry
fi
