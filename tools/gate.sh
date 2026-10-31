#!/usr/bin/env bash
# The merge gate of this repository: every check a change must pass, in order, stopping at the first failure.
#
#   tools/gate.sh
#
# Steps: ignored-audit (every #[ignore]d test has a step that runs it), fuzz-check (the fuzz targets compile), clippy, clippy-radovid, clippy-ui, wasm, wasm-radovid, wasm-web, test (profile test-release), test-product (bota_proto as the product builds), test-ui, build-server and test-radovid (bota_radovid's end-to-end tests), bench (the dummy bench's digest
# checks), deny, import (the item importer, tools/import_items: its own checks and, with a Dota 2 install, that the
# committed item table is what the install gives; no install fails the gate), then fmt and import-fmt (last: they change no code).
# Environment:
#   GATE_STEP_LOG=FILE       append the seconds each step took to FILE (tab separated)
#   GATE_PRECHECKED=1        start although tools/precheck.sh has not passed this tree
#   GATE_AGAIN=1             run although this tree already passed
#   GATE_SKIP="deny bench"   steps left out (to iterate on one; the gate fails with them left out)
#   GATE_LOGS=DIR            where each step's output goes (default: temp/gate under the repository)
# Prints GATE-OK when every step ran and passed, GATE-FAIL <step> (with the tail of its output) otherwise.
set -uo pipefail
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
cd "$here" || exit 2
logs=${GATE_LOGS:-$here/temp/gate}
mkdir -p "$logs"
# shellcheck source=tools/stamp.sh
. "$here/tools/stamp.sh"
# The gate confirms; the precheck finds. It starts only on a tree tools/precheck.sh has passed, and not again on a tree that
# already passed (formatting is left out of both). GATE_PRECHECKED=1 and GATE_AGAIN=1 skip the two checks, for a head that is
# gated on behalf of something else (a merge, a consumer check) and was checked elsewhere.
tree=$(tree_key)
if [ "${GATE_PRECHECKED:-0}" != 1 ] && [ -z "$(stamp_has "$tree" precheck)" ]; then
    echo "gate: no passing precheck for this tree. Run tools/precheck.sh first (it checks what the change touches, in minutes; this gate is the confirmation, not the place to find errors)." >&2
    exit 3
fi
if [ "${GATE_AGAIN:-0}" != 1 ] && [ -n "$(stamp_has "$tree" gate)" ]; then
    echo "gate: this tree already passed the gate ($(stamp_has "$tree" gate)). GATE_AGAIN=1 runs it again." >&2
    exit 3
fi
if only_docs_changed "${GATE_BASE:-main}"; then
    echo "GATE-OK (documents only: nothing to confirm)"
    stamp_put "$tree" "$mode"
    exit 0
fi
skip=" ${GATE_SKIP:-} "
skipped=0

installs() { # the Dota 2 installs the importer would find, as heatel looks for them
    local library
    for library in "$HOME/.var/app/com.valvesoftware.Steam/.local/share/Steam" "$HOME/.steam/steam" \
        "$HOME/.local/share/Steam"; do
        [ -f "$library/steamapps/common/dota 2 beta/game/dota/pak01_dir.vpk" ] && echo "$library"
    done
}

step() { # name command...
    local name=$1
    shift
    case "$skip" in *" $name "*) echo "== $name: skipped"; skipped=1; return 0 ;; esac
    echo "== $name"
    local began=$SECONDS
    "$@" > "$logs/$name.log" 2>&1
    local rc=$?
    # GATE_STEP_LOG=FILE: one line per step (time, repository directory, step, seconds, exit code), to see where a gate spends its minutes
    [ -z "${GATE_STEP_LOG:-}" ] || printf '%s\t%s\t%s\t%s\t%s\n' "$(date +%FT%T)" "$(basename -- "$here")" "$name" "$((SECONDS - began))" "$rc" >> "$GATE_STEP_LOG"
    if [ $rc -ne 0 ]; then
        tail -n 40 "$logs/$name.log"
        echo "GATE-FAIL $name (full output: $logs/$name.log)"
        exit 1
    fi
    grep -E '^test result' "$logs/$name.log" | awk '{p += $4; f += $6; i += $8} END {if (NR) print "   " p " passed, " f " failed, " i " ignored"}'
}

step ignored-audit tools/ignored_audit.sh
step fuzz-check fuzz/fuzz.sh check
step clippy cargo clippy --all-targets -- -D warnings
step clippy-radovid cargo clippy -p bota_radovid --all-targets --features e2e -- -D warnings
step clippy-ui cargo clippy -p bota_client_core --all-targets --features ui -- -D warnings
step wasm cargo check --target wasm32-unknown-unknown -p bota_client_core -p bota_proto --features bota_client_core/ui
step wasm-radovid cargo check --target wasm32-unknown-unknown -p bota_radovid --features web
step wasm-web cargo check --target wasm32-unknown-unknown -p bota_client_core --features bota_client_core/web
# Tests are built as the tests-with-checks profile of Cargo.toml (release optimisation, overflow checks and debug assertions): no debug build.
step test cargo test --all --profile test-release
# The product has no overflow checks: its arithmetic saturates (a test in bota_proto that only builds without debug assertions).
step test-product cargo test -p bota_proto --release
step test-ui cargo test -p bota_client_core --features ui --profile test-release
# bota_radovid is empty without its features: its tests (a real radovid in-process launching the release bota_server) run here.
step build-server cargo build --release -p bota_server
step test-radovid env BOTA_SERVER_BIN="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/release/bota_server" cargo test -p bota_radovid --features e2e --profile test-release
step bench cargo bench -q -p bota_server --bench dummy -- --test
step deny cargo deny check

# The importer reads the install with heatel, a checkout beside this one.
importer=(--manifest-path tools/import_items/Cargo.toml)
step import-clippy cargo clippy "${importer[@]}" --all-targets -- -D warnings
if [ -z "$(installs)" ]; then
    echo "GATE-FAIL import (no Dota 2 install: the committed item table cannot be checked against it)"
    exit 1
fi
step import cargo test --release "${importer[@]}" -- --include-ignored
# Formatting changes no code, so it comes last: a gate that fails only here needs `cargo fmt` and an amend, not a rerun.
step fmt cargo fmt --all --check
step import-fmt cargo fmt "${importer[@]}" --check

if [ "$skipped" = 1 ]; then
    echo "GATE-FAIL steps were skipped (GATE_SKIP): that is not a pass"
    exit 1
fi
echo "GATE-OK"
stamp_put "$tree" gate
