#!/usr/bin/env bash
# The fast check to run while working, before tools/gate.sh: it stops at the first thing that is wrong.
#
#   tools/precheck.sh [BASE]     BASE: the branch the work is compared with (default: main)
#
# Formatting is no part of it: it changes no code, and is checked last, by the gate.
# Steps, each only for what the work touches (the files changed since the merge-base with BASE, committed or not):
#   clippy  the crates with a changed file and every crate of the repository that depends on them
#   test    the tests of those crates; the tests that failed in the last run go first, and the run stops at the first failure
#   wasm    the browser build checks, only when a crate they cover changed
# A warm checkout takes a few minutes at most for a typical change. The gate stays the confirmation: a failure there that
# this would have caught is a process bug. PRECHECK_ALL=1 takes every crate. Failed test names are kept in
# $(git rev-parse --git-path precheck-failed) and cleared by a green run. A green run stamps the tree (tools/stamp.sh), and
# tools/gate.sh starts only on a stamped tree.
set -uo pipefail
cd -- "$(git rev-parse --show-toplevel)" || exit 2
# shellcheck source=tools/stamp.sh
. "$(dirname -- "${BASH_SOURCE[0]}")/stamp.sh"
base=${1:-${PRECHECK_BASE:-main}}
# crates whose change the browser checks cover, and the checks (the same as tools/gate.sh)
wasm_crates="bota_client_core bota_proto"
wasm_checks=(
    "cargo check --target wasm32-unknown-unknown -p bota_client_core -p bota_proto --features bota_client_core/ui"
    "cargo check --target wasm32-unknown-unknown -p bota_client_core --features bota_client_core/web"
)
# per crate, extra clippy runs (a feature the plain run does not build)
extra_clippy() { # crate
    case $1 in bota_client_core) echo "cargo clippy -p bota_client_core --all-targets --features ui -- -D warnings" ;; esac
}
extra_test() { # crate
    case $1 in bota_client_core) echo "cargo test -p bota_client_core --features ui --profile test-release" ;; esac
}

record=$(git rev-parse --git-path precheck-failed)
started=$SECONDS
step() { local name=$1 t=$SECONDS; shift; echo "== $name"; "$@" || { echo "PRECHECK-FAIL $name ($((SECONDS - t)) s)"; return 1; }; echo "   $name $((SECONDS - t)) s"; }

mb=$(git merge-base HEAD "$base" 2>/dev/null || git rev-parse HEAD)
changed=$( { git diff --name-only "$mb"; git ls-files --others --exclude-standard; } | sort -u)
plan_out=$(PRECHECK_ALL=${PRECHECK_ALL:-0} CHANGED="$changed" python3 - <<'PY'
import json, os, subprocess
meta = json.loads(subprocess.run(["cargo", "metadata", "--format-version", "1", "--offline"], capture_output=True, text=True, check=True).stdout)
root = meta["workspace_root"].rstrip("/") + "/"
ids = set(meta["workspace_members"])
pkgs = {p["id"]: p for p in meta["packages"] if p["id"] in ids}
dirs = {i: os.path.relpath(os.path.dirname(p["manifest_path"]), meta["workspace_root"]).replace(".", "", 1) if os.path.dirname(p["manifest_path"]) == meta["workspace_root"] else os.path.relpath(os.path.dirname(p["manifest_path"]), meta["workspace_root"]) for i, p in pkgs.items()}
name = {i: p["name"] for i, p in pkgs.items()}
hit = set()
files = [f for f in os.environ["CHANGED"].split("\n") if f]
everything = os.environ["PRECHECK_ALL"] == "1" or any(f in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "deny.toml") or f.startswith(".cargo/") for f in files)
if everything:
    hit = set(pkgs)
else:
    for f in files:
        if f.endswith(".md") or f.startswith(("tools/", "docs/", "fuzz/")):
            continue
        best = max((i for i in pkgs if dirs[i] == "" or f == dirs[i] or f.startswith(dirs[i] + "/")), key=lambda i: len(dirs[i]), default=None)
        if best: hit.add(best)
# reverse dependents among the crates of the repository
deps = {i: set() for i in pkgs}
for n in meta["resolve"]["nodes"]:
    if n["id"] in pkgs:
        deps[n["id"]] = {d["pkg"] for d in n["deps"] if d["pkg"] in pkgs}
todo = list(hit)
while todo:
    cur = todo.pop()
    for i, ds in deps.items():
        if cur in ds and i not in hit:
            hit.add(i); todo.append(i)
print(" ".join(sorted(name[i] for i in hit)))
# a crate at the root of the repository: tests are picked by module from the changed source files, "all" when one cannot be
root_pkgs = [i for i in pkgs if dirs[i] == ""]
filt = []
if root_pkgs and hit == {root_pkgs[0]} and not everything:
    for f in files:
        if f.endswith(".md") or f.startswith(("tools/", "docs/", "fuzz/")): continue
        if f.startswith("src/") and f.endswith(".rs") and os.path.basename(f) not in ("main.rs", "lib.rs"):
            mod = f[4:-3].replace("/", "::")
            filt.append(mod[:-5] + "::" if mod.endswith("::mod") else mod)
        elif f.startswith("tests/") and f.endswith(".rs"):
            continue
        else:
            filt = ["*"]; break
    if "*" in filt: filt = []
print(" ".join(sorted(set(filt))))
PY
)
plan=$(sed -n 1p <<< "$plan_out")
filters=$(sed -n 2p <<< "$plan_out")
if [ -z "$plan" ]; then echo "precheck: no crate touched"; echo "PRECHECK-OK ($((SECONDS - started)) s)"; exit 0; fi
echo "precheck: crates $plan"
pflags=(); for c in $plan; do pflags+=(-p "$c"); done

step clippy cargo clippy "${pflags[@]}" --all-targets -- -D warnings || exit 1
for c in $plan; do
    e=$(extra_clippy "$c")
    [ -z "$e" ] || step "clippy ($c)" bash -c "$e" || exit 1
done

run_tests() { # the failed ones first, then everything of the touched crates
    local log; log=$(mktemp)
    if [ -s "$record" ]; then
        echo "   first, the tests that failed last time"
        # shellcheck disable=SC2046
        cargo test --profile test-release "${pflags[@]}" -- $(cat "$record") 2>&1 | tee "$log"
        if [ "${PIPESTATUS[0]}" -ne 0 ]; then rm -f "$log"; return 1; fi
    fi
    cargo test --profile test-release "${pflags[@]}" -- $filters 2>&1 | tee "$log"
    local rc=${PIPESTATUS[0]}
    if [ "$rc" -ne 0 ]; then
        sed -n 's/^---- \(.*\) stdout ----$/\1/p' "$log" | sort -u > "$record"
    else
        rm -f "$record"
    fi
    rm -f "$log"
    return "$rc"
}
step test run_tests || exit 1
for c in $plan; do
    e=$(extra_test "$c"); [ -z "$e" ] || step "test ($c)" bash -c "$e" || exit 1
done

for c in $plan; do
    case " $wasm_crates " in *" $c "*)
        for w in "${wasm_checks[@]}"; do step wasm bash -c "$w" || exit 1; done
        break ;;
    esac
done
stamp_put "$(tree_key)" precheck
echo "PRECHECK-OK ($((SECONDS - started)) s)"
