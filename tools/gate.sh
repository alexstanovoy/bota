#!/usr/bin/env bash
# The merge gate of this repository: every check a change must pass, in order, stopping at the first failure.
#
#   tools/gate.sh
#
# Steps: fmt, clippy, clippy-ui, wasm, wasm-web, test (debug), test-release, test-ui, bench (the dummy bench's digest
# checks), deny.
# Environment:
#   GATE_SKIP="deny bench"   steps left out (to iterate on one; the gate only counts as passed with none skipped)
#   GATE_LOGS=DIR            where each step's output goes (default: temp/gate under the repository)
# Prints GATE-OK when every step ran and passed, GATE-FAIL <step> (with the tail of its output) otherwise.
set -uo pipefail
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
cd "$here" || exit 2
logs=${GATE_LOGS:-$here/temp/gate}
mkdir -p "$logs"
skip=" ${GATE_SKIP:-} "
skipped=0

step() { # name command...
    local name=$1
    shift
    case "$skip" in *" $name "*) echo "== $name: skipped"; skipped=1; return 0 ;; esac
    echo "== $name"
    if ! "$@" > "$logs/$name.log" 2>&1; then
        tail -n 40 "$logs/$name.log"
        echo "GATE-FAIL $name (full output: $logs/$name.log)"
        exit 1
    fi
    grep -E '^test result' "$logs/$name.log" | awk '{p += $4; f += $6; i += $8} END {if (NR) print "   " p " passed, " f " failed, " i " ignored"}'
}

step fmt cargo fmt --all --check
step clippy cargo clippy --all-targets -- -D warnings
step clippy-ui cargo clippy -p bota_client_core --all-targets --features ui -- -D warnings
step wasm cargo check --target wasm32-unknown-unknown -p bota_client_core -p bota_proto --features bota_client_core/ui
step wasm-web cargo check --target wasm32-unknown-unknown -p bota_client_core --features bota_client_core/web
step test cargo test --all
step test-release cargo test --all --release
step test-ui cargo test -p bota_client_core --features ui
step bench cargo bench -q -p bota_server --bench dummy -- --test
step deny cargo deny check

if [ "$skipped" = 1 ]; then
    echo "GATE-OK (with skipped steps: not a pass)"
else
    echo "GATE-OK"
fi
