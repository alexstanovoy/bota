#!/usr/bin/env bash
# The fast check to run while working, before tools/gate.sh: it stops at the first thing that is wrong.
#
#   tools/precheck.sh [BASE]     BASE: what the work is compared with (default: origin/main, main without an origin)
#
# Formatting is no part of it: it changes no code, and is checked last, by the gate.
# Steps, each only for what the work touches (the files changed since the merge-base with BASE, committed or not; what that
# affects is tools/test_groups.py's plan, the same as the gate's):
#   clippy       the crates with a changed file and every crate of the repository that depends on them
#   test-groups  every test in exactly one group and every file routed (the gate's check)
#   test         the test groups the change runs; the groups that failed in the last run go first, and the run stops at the
#                first failure
#   wasm         the browser build checks, only when a crate they cover changed
# A warm checkout takes a few minutes at most for a typical change. The gate stays the confirmation: a failure there that
# this would have caught is a process bug. PRECHECK_ALL=1 takes everything. The groups that failed are kept in
# $(git rev-parse --git-path precheck-failed) and cleared by a green run. A green run stamps the tree (tools/stamp.sh), and
# tools/gate.sh starts only on a stamped tree.
set -uo pipefail
cd -- "$(git rev-parse --show-toplevel)" || exit 2
# shellcheck source=tools/stamp.sh
. "$(dirname -- "${BASH_SOURCE[0]}")/stamp.sh"
base=${1:-${PRECHECK_BASE:-$(default_base)}}
if [ "${PRECHECK_ALL:-0}" = 1 ]; then selection=--all; else selection="--base $base"; fi

record=$(git rev-parse --git-path precheck-failed)
started=$SECONDS
step() { local name=$1 t=$SECONDS; shift; echo "== $name"; "$@" || { echo "PRECHECK-FAIL $name ($((SECONDS - t)) s)"; return 1; }; echo "   $name $((SECONDS - t)) s"; }
has() { case " $1 " in *" $2 "*) return 0 ;; esac; return 1; }

# shellcheck disable=SC2086
plan=$(python3 tools/test_groups.py plan $selection) || { echo "PRECHECK-FAIL plan" >&2; exit 2; }
groups=$(sed -n 's/^groups *//p' <<< "$plan")
crates=$(sed -n 's/^crates *//p' <<< "$plan")
scope=$(sed -n 's/^scope //p' <<< "$plan")
# the stamp names the tree checked and the selection it was checked with (tools/stamp.sh)
tree=$(tree_key)
if [ -z "$groups" ] && [ -z "$crates" ]; then
    echo "precheck: no crate and no test group touched"
    stamp_put "$tree-$scope" precheck
    echo "PRECHECK-OK ($((SECONDS - started)) s)"
    exit 0
fi
echo "precheck: crates ${crates:-none}; groups ${groups:-none}"

lint=(); for c in $crates; do [ "$c" = bota_radovid ] || lint+=(-p "$c"); done
if [ ${#lint[@]} -gt 0 ]; then step clippy cargo clippy "${lint[@]}" --all-targets -- -D warnings || exit 1; fi
if has "$crates" bota_radovid; then
    step "clippy (bota_radovid)" cargo clippy -p bota_radovid --all-targets --features e2e -- -D warnings || exit 1
fi
if has "$crates" bota_client_core; then
    step "clippy (bota_client_core ui)" cargo clippy -p bota_client_core --all-targets --features ui -- -D warnings || exit 1
fi
# shellcheck disable=SC2086
step test-groups python3 tools/test_groups.py check $selection || exit 1

run_groups() { # the groups that failed last time first, then the rest
    local first=() rest=() g
    for g in $groups; do
        if [ -s "$record" ] && grep -qx "$g" "$record"; then first+=("$g"); else rest+=("$g"); fi
    done
    if [ ${#first[@]} -gt 0 ]; then
        echo "   first, the groups that failed last time: ${first[*]}"
        python3 tools/test_groups.py run --record "$record" "${first[@]}" || return 1
    fi
    if [ ${#rest[@]} -gt 0 ]; then python3 tools/test_groups.py run --record "$record" "${rest[@]}" || return 1; fi
    rm -f "$record"
}
if [ -n "$groups" ]; then step test run_groups || exit 1; fi

if has "$crates" bota_client_core; then
    step wasm cargo check --target wasm32-unknown-unknown -p bota_client_core -p bota_proto --features bota_client_core/ui || exit 1
    step "wasm (web)" cargo check --target wasm32-unknown-unknown -p bota_client_core --features bota_client_core/web || exit 1
fi
if has "$crates" bota_radovid; then
    step "wasm (bota_radovid)" cargo check --target wasm32-unknown-unknown -p bota_radovid --features web || exit 1
fi
if [ "$(tree_key)" != "$tree" ]; then
    echo "PRECHECK-FAIL the tree changed while the precheck ran (git status --short):"
    git status --short
    exit 1
fi
stamp_put "$tree-$scope" precheck
echo "PRECHECK-OK ($((SECONDS - started)) s)"
