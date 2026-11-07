#!/usr/bin/env bash
# The fast check to run while working, before tools/gate.sh: it stops at the first thing that is wrong.
#
#   tools/precheck.sh [BASE]     BASE: what the work is compared with (default: origin/main, main without an origin)
#
# Formatting is no part of it: it changes no code, and is checked last, by the gate.
# Steps, each only for what the work touches (the files changed since the merge-base with BASE, committed or not; what that
# affects is tools/test_groups.py's plan, the same as the gate's):
#   clippy       the crates with a changed file and every crate of the repository that depends on them; the bot-script host
#                (crates/bota_lua, a workspace of its own: clippy-lua, fuzz-lua, wasm-lua and the test group lua) too where the
#                luagee checkout beside this repository is. Without it the precheck says "not covered" (also in PRECHECK-OK)
#                and refuses a change to its files; TEST_GROUPS_REQUIRE_ALL=1 makes a missing checkout a failure
#   test-groups  every test in exactly one group and every file routed (the gate's check)
#   fuzz-check   the fuzz targets compile (compile only, as in the gate), when a crate they build on or they changed
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
# shellcheck source=tools/steps.sh
. "$(dirname -- "${BASH_SOURCE[0]}")/steps.sh"
base=${1:-${PRECHECK_BASE:-$(default_base)}}
if [ "${PRECHECK_ALL:-0}" = 1 ]; then selection=--all; else selection="--base $base"; fi

record=$(git rev-parse --git-path precheck-failed)
started=$SECONDS
step() { local name=$1 t=$SECONDS; shift; echo "== $name"; "$@" < /dev/null || { echo "PRECHECK-FAIL $name ($((SECONDS - t)) s)"; return 1; }; echo "   $name $((SECONDS - t)) s"; }
run_steps() { # the lines of lint_steps, fuzz_steps or wasm_steps (tools/steps.sh) on stdin: the needed steps
    local name need reason command words
    while IFS=$'\t' read -r name need reason command; do
        read -ra words <<< "$command"
        [ "$need" = 1 ] || continue
        step "$name" "${words[@]}" || return 1
    done
}

# shellcheck disable=SC2086
plan=$(python3 tools/test_groups.py plan $selection) || { echo "PRECHECK-FAIL plan" >&2; exit 2; }
groups=$(sed -n 's/^groups *//p' <<< "$plan")
crates=$(sed -n 's/^crates *//p' <<< "$plan")
fuzz=$(sed -n 's/^fuzz //p' <<< "$plan")
scope=$(sed -n 's/^scope //p' <<< "$plan")
uncovered=$(sed -n 's/^uncovered *//p' <<< "$plan") # crates whose required checkout is absent: nothing here covers them
# the stamp names the tree checked and the selection it was checked with (tools/stamp.sh)
tree=$(tree_key) || exit 2
if [ -z "$groups" ] && [ -z "$crates" ] && [ "$fuzz" != 1 ]; then
    echo "precheck: no crate, no test group and no fuzz target touched"
    stamp_put "$tree-$scope" precheck
    echo "PRECHECK-OK ($((SECONDS - started)) s${uncovered:+; not covered: $uncovered})"
    exit 0
fi
echo "precheck: crates ${crates:-none}; groups ${groups:-none}; fuzz targets $([ "$fuzz" = 1 ] && echo checked || echo untouched)"
[ -z "$uncovered" ] || echo "precheck: NOT COVERED here: $uncovered (a checkout they need is absent; the change does not touch their files)"

run_steps < <(lint_steps "$crates") || exit 1
# shellcheck disable=SC2086
step test-groups python3 tools/test_groups.py check $selection || exit 1
run_steps < <(fuzz_steps "$fuzz") || exit 1

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

run_steps < <(wasm_steps "$crates") || exit 1
now=$(tree_key) || exit 2
if [ "$now" != "$tree" ]; then
    echo "PRECHECK-FAIL the tree changed while the precheck ran (git status --short):"
    git status --short
    exit 1
fi
stamp_put "$tree-$scope" precheck
echo "PRECHECK-OK ($((SECONDS - started)) s${uncovered:+; not covered: $uncovered})"
