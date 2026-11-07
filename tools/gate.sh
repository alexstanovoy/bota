#!/usr/bin/env bash
# The merge gate of this repository: every check a change must pass, in order, stopping at the first failure.
#
#   tools/gate.sh                what the change can affect: the files changed since the merge-base with origin/main
#   GATE_FULL=1 tools/gate.sh    everything, whatever changed: for releases and after a batch of changes
#
# What a change affects is tools/test_groups.py's plan (the groups and routes are tools/test_groups.toml): the test groups
# its files run, and the crates to lint (the changed ones and every crate building on them). A step the change cannot affect
# is named with the reason; none is left out quietly.
# Steps: ignored-audit (every #[ignore]d test has a step that runs it), test-groups (every test in exactly one group, every
# file routed, at most 30 groups), fuzz-check (the fuzz targets compile), clippy, clippy-radovid, clippy-lua, fuzz-lua,
# clippy-ui, wasm, wasm-lua, wasm-radovid, wasm-web, test (the groups: their tests in profile test-release, bota_proto as the product builds it,
# bota_radovid's end-to-end tests against the release bota_server, the dummy bench's digest checks, the item importer's
# checks against a Dota 2 install, which must be there when they run), deny, deny-lua, import-clippy, then fmt, fmt-lua and
# import-fmt (last: they change no code).
# The bot-script host (crates/bota_lua: clippy-lua, fuzz-lua, wasm-lua, deny-lua, fmt-lua and the test group lua) needs the
# luagee checkout beside this repository and is a workspace of its own. Without that checkout the gate leaves it out and says
# so ("NOT COVERED", and in GATE-OK), but refuses a change to its files; TEST_GROUPS_REQUIRE_ALL=1 makes a missing checkout
# a failure for every change, for the machines that must cover it.
# Environment:
#   TEST_GROUPS_REQUIRE_ALL=1  fail where the luagee checkout is absent instead of leaving the bot-script host out
#   GATE_FULL=1              everything (a full pass also stands for the affected one)
#   GATE_BASE=REV            what the change is measured against (default origin/main, main without an origin)
#   GATE_STEP_LOG=FILE       append the seconds each step took to FILE (tab separated)
#   GATE_PRECHECKED=1        start although tools/precheck.sh has not passed this tree
#   GATE_AGAIN=1             run although this tree already passed
#   GATE_SKIP="deny test"    steps left out (to iterate on one; the gate fails with them left out)
#   GATE_LOGS=DIR            where each step's output goes (default: temp/gate under the repository)
# Prints GATE-OK when every step ran and passed, GATE-FAIL <step> (with the tail of its output) otherwise.
set -uo pipefail
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
cd "$here" || exit 2
logs=${GATE_LOGS:-$here/temp/gate}
mkdir -p "$logs"
# shellcheck source=tools/stamp.sh
. "$here/tools/stamp.sh"
# shellcheck source=tools/steps.sh
. "$here/tools/steps.sh"
base=${GATE_BASE:-$(default_base)}
if [ "${GATE_FULL:-0}" = 1 ]; then mode=gate-full what=full selection=--all; else mode=gate what=affected selection="--base $base"; fi
# shellcheck disable=SC2086
if ! plan=$(python3 tools/test_groups.py plan $selection); then
    echo "GATE-FAIL plan (tools/test_groups.py plan $selection)" >&2
    exit 2
fi
groups=$(sed -n 's/^groups *//p' <<< "$plan")
crates=$(sed -n 's/^crates *//p' <<< "$plan")
fuzz=$(sed -n 's/^fuzz //p' <<< "$plan")
scope=$(sed -n 's/^scope //p' <<< "$plan")
uncovered=$(sed -n 's/^uncovered *//p' <<< "$plan") # crates whose required checkout is absent: nothing here covers them
optional=$(python3 tools/test_groups.py available) # the crates of their own whose sibling checkouts are here (bota_lua)
# The gate confirms; the precheck finds. It starts only on a tree tools/precheck.sh has passed with the same selection (or
# in full), and not again on a tree that already passed (formatting is left out of both; a full pass stands for an
# affected one too). A stamp names the tree and the selection: what it was compared with and what that selected.
# GATE_PRECHECKED=1 and GATE_AGAIN=1 skip the two checks, for a head that is gated on behalf of something else (a merge,
# a consumer check) and was checked elsewhere.
tree=$(tree_key) || exit 2
key=$tree-$scope
if [ "${GATE_PRECHECKED:-0}" != 1 ] && [ -z "$(stamp_has "$key" precheck)$(stamp_has "$tree-full" precheck)" ]; then
    echo "gate: no passing precheck for this tree and selection ($what). Run tools/precheck.sh first (it checks what the change touches, in minutes; this gate is the confirmation, not the place to find errors)." >&2
    exit 3
fi
passed=$(stamp_has "$tree-full" gate-full)
[ "$mode" = gate-full ] || passed=${passed:-$(stamp_has "$key" gate)}
if [ "${GATE_AGAIN:-0}" != 1 ] && [ -n "$passed" ]; then
    echo "gate: this tree already passed the gate ($passed). GATE_AGAIN=1 runs it again." >&2
    exit 3
fi
if [ "$mode" = gate ] && only_docs_changed "$base"; then
    echo "GATE-OK (documents only: nothing to confirm${uncovered:+; not covered: $uncovered})"
    stamp_put "$key" "$mode"
    exit 0
fi
echo "gate: $what ($selection): groups ${groups:-none}; crates ${crates:-none}"
[ -z "$uncovered" ] || echo "gate: NOT COVERED here: $uncovered (a checkout they need is absent; the change does not touch their files)"
skip=" ${GATE_SKIP:-} "
# leaving anything out, even a step this change does not run or one that does not exist, is not a pass
skipped=0
[ -z "${skip// /}" ] || skipped=1

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
    "$@" > "$logs/$name.log" 2>&1 < /dev/null
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

step_if() { # condition reason name command...: the step when the condition holds, its name and the reason otherwise
    local condition=$1 reason=$2
    shift 2
    if eval "$condition"; then step "$@"; else echo "== $1: not affected ($reason)"; fi
}

run_steps() { # the lines of lint_steps or wasm_steps (tools/steps.sh) on stdin: the needed steps run, the others are named
    local name need reason command words
    while IFS=$'\t' read -r name need reason command; do
        read -ra words <<< "$command"
        if [ "$need" = 1 ]; then step "$name" "${words[@]}"; else echo "== $name: not affected ($reason)"; fi
    done
}

step ignored-audit tools/ignored_audit.sh
# shellcheck disable=SC2086
step test-groups python3 tools/test_groups.py check $selection
run_steps < <(fuzz_steps "$fuzz")
run_steps < <(lint_steps "$crates")
run_steps < <(wasm_steps "$crates")
if has "$groups" import_items && [ -z "$(installs)" ]; then
    echo "GATE-FAIL test (group import_items runs, and there is no Dota 2 install: the committed item table cannot be checked against it)"
    exit 1
fi
# shellcheck disable=SC2086
step_if '[ -n "$groups" ]' "no test group" test python3 tools/test_groups.py run $groups
step deny cargo deny check
step_if 'has "$optional" bota_lua' "no luagee checkout" deny-lua cargo deny --config deny.toml --manifest-path "$botscript" check

# The importer reads the install with heatel, a checkout beside this one.
importer=(--manifest-path tools/import_items/Cargo.toml)
step_if 'has "$groups" import_items' "group import_items does not run" import-clippy cargo clippy "${importer[@]}" --all-targets -- -D warnings
# Formatting changes no code, so it comes last: a gate that fails only here needs `cargo fmt` and an amend, not a rerun.
step fmt cargo fmt --all --check
step_if 'has "$optional" bota_lua' "no luagee checkout" fmt-lua cargo fmt --manifest-path "$botscript" --all --check
step import-fmt cargo fmt "${importer[@]}" --check

if [ "$skipped" = 1 ]; then
    echo "GATE-FAIL steps were skipped (GATE_SKIP): that is not a pass"
    exit 1
fi
# What passed is what was checked: a tree changed by a step or by hand meanwhile gets no stamp.
now=$(tree_key) || exit 2
if [ "$now" != "$tree" ]; then
    echo "GATE-FAIL the tree changed while the gate ran (git status --short):"
    git status --short
    exit 1
fi
echo "GATE-OK ($what: $(wc -w <<< "$groups") groups${uncovered:+; not covered: $uncovered})"
stamp_put "$key" "$mode"
