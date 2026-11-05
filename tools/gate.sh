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
# file routed, at most 30 groups), fuzz-check (the fuzz targets compile), clippy, clippy-radovid, clippy-ui, wasm,
# wasm-radovid, wasm-web, test (the groups: their tests in profile test-release, bota_proto as the product builds it,
# bota_radovid's end-to-end tests against the release bota_server, the dummy bench's digest checks, the item importer's
# checks against a Dota 2 install, which must be there when they run), deny, import-clippy, then fmt and import-fmt (last:
# they change no code).
# Environment:
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
base=${GATE_BASE:-$(default_base)}
if [ "${GATE_FULL:-0}" = 1 ]; then mode=gate-full what=full scope=--all; else mode=gate what=affected scope="--base $base"; fi
# The gate confirms; the precheck finds. It starts only on a tree tools/precheck.sh has passed, and not again on a tree that
# already passed (formatting is left out of both; a full pass stands for an affected one too). GATE_PRECHECKED=1 and
# GATE_AGAIN=1 skip the two checks, for a head that is gated on behalf of something else (a merge, a consumer check) and was
# checked elsewhere.
tree=$(tree_key)
if [ "${GATE_PRECHECKED:-0}" != 1 ] && [ -z "$(stamp_has "$tree" precheck)" ]; then
    echo "gate: no passing precheck for this tree. Run tools/precheck.sh first (it checks what the change touches, in minutes; this gate is the confirmation, not the place to find errors)." >&2
    exit 3
fi
passed=$(stamp_has "$tree" gate-full)
[ "$mode" = gate-full ] || passed=${passed:-$(stamp_has "$tree" gate)}
if [ "${GATE_AGAIN:-0}" != 1 ] && [ -n "$passed" ]; then
    echo "gate: this tree already passed the gate ($passed). GATE_AGAIN=1 runs it again." >&2
    exit 3
fi
if [ "$mode" = gate ] && only_docs_changed "$base"; then
    echo "GATE-OK (documents only: nothing to confirm)"
    stamp_put "$tree" "$mode"
    exit 0
fi
# shellcheck disable=SC2086
if ! plan=$(python3 tools/test_groups.py plan $scope); then
    echo "GATE-FAIL plan (tools/test_groups.py plan $scope)"
    exit 1
fi
groups=$(sed -n 's/^groups *//p' <<< "$plan")
crates=$(sed -n 's/^crates *//p' <<< "$plan")
fuzz=$(sed -n 's/^fuzz //p' <<< "$plan")
echo "gate: $what ($scope): groups ${groups:-none}; crates ${crates:-none}"
skip=" ${GATE_SKIP:-} "
skipped=0

installs() { # the Dota 2 installs the importer would find, as heatel looks for them
    local library
    for library in "$HOME/.var/app/com.valvesoftware.Steam/.local/share/Steam" "$HOME/.steam/steam" \
        "$HOME/.local/share/Steam"; do
        [ -f "$library/steamapps/common/dota 2 beta/game/dota/pak01_dir.vpk" ] && echo "$library"
    done
}

has() { # list word: success when word is in the space separated list
    case " $1 " in *" $2 "*) return 0 ;; esac
    return 1
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

step_if() { # condition reason name command...: the step when the condition holds, its name and the reason otherwise
    local condition=$1 reason=$2
    shift 2
    if eval "$condition"; then step "$@"; else echo "== $1: not affected ($reason)"; fi
}

step ignored-audit tools/ignored_audit.sh
# shellcheck disable=SC2086
step test-groups python3 tools/test_groups.py check $scope
step_if '[ "$fuzz" = 1 ]' "no crate and no fuzz target changed" fuzz-check fuzz/fuzz.sh check
lint=(); for c in $crates; do [ "$c" = bota_radovid ] || lint+=(-p "$c"); done
step_if '[ ${#lint[@]} -gt 0 ]' "no crate changed" clippy cargo clippy "${lint[@]}" --all-targets -- -D warnings
step_if 'has "$crates" bota_radovid' "bota_radovid and what it builds on are unchanged" clippy-radovid cargo clippy -p bota_radovid --all-targets --features e2e -- -D warnings
step_if 'has "$crates" bota_client_core' "bota_client_core and what it builds on are unchanged" clippy-ui cargo clippy -p bota_client_core --all-targets --features ui -- -D warnings
step_if 'has "$crates" bota_client_core' "bota_client_core and what it builds on are unchanged" wasm cargo check --target wasm32-unknown-unknown -p bota_client_core -p bota_proto --features bota_client_core/ui
step_if 'has "$crates" bota_radovid' "bota_radovid and what it builds on are unchanged" wasm-radovid cargo check --target wasm32-unknown-unknown -p bota_radovid --features web
step_if 'has "$crates" bota_client_core' "bota_client_core and what it builds on are unchanged" wasm-web cargo check --target wasm32-unknown-unknown -p bota_client_core --features bota_client_core/web
if has "$groups" import_items && [ -z "$(installs)" ]; then
    echo "GATE-FAIL test (group import_items runs, and there is no Dota 2 install: the committed item table cannot be checked against it)"
    exit 1
fi
# shellcheck disable=SC2086
step_if '[ -n "$groups" ]' "no test group" test python3 tools/test_groups.py run $groups
step deny cargo deny check

# The importer reads the install with heatel, a checkout beside this one.
importer=(--manifest-path tools/import_items/Cargo.toml)
step_if 'has "$groups" import_items' "group import_items does not run" import-clippy cargo clippy "${importer[@]}" --all-targets -- -D warnings
# Formatting changes no code, so it comes last: a gate that fails only here needs `cargo fmt` and an amend, not a rerun.
step fmt cargo fmt --all --check
step import-fmt cargo fmt "${importer[@]}" --check

if [ "$skipped" = 1 ]; then
    echo "GATE-FAIL steps were skipped (GATE_SKIP): that is not a pass"
    exit 1
fi
echo "GATE-OK ($what: $(wc -w <<< "$groups") groups)"
stamp_put "$tree" "$mode"
