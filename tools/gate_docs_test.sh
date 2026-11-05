#!/usr/bin/env bash
# The documents-only path of tools/gate.sh, end to end in a disposable repository: a branch that changes only a document
# passes at once (exit 0, GATE-OK) and stamps its tree as gated. It once printed GATE-OK and then died on an unset
# variable, unstamped.
#
#   tools/gate_docs_test.sh
set -euo pipefail
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT
cd "$scratch"
git init -q -b main
git config user.email gate-test@example.invalid
git config user.name gate-test
mkdir tools
cp "$here/tools/gate.sh" "$here/tools/stamp.sh" tools/
git add -A && git commit -qm base
git switch -q -c docs
echo "A change of words." > NOTES.md
git add -A && git commit -qm docs
out=$(env -u mode -u GATE_LOGS -u GATE_STEP_LOG -u GATE_SKIP -u GATE_AGAIN -u GATE_BASE GATE_PRECHECKED=1 \
    bash tools/gate.sh 2>&1) || { echo "gate_docs_test: the gate failed on a documents-only change:"; echo "$out"; exit 1; }
grep -q '^GATE-OK (documents only' <<< "$out" || { echo "gate_docs_test: no GATE-OK:"; echo "$out"; exit 1; }
# shellcheck source=tools/stamp.sh
. tools/stamp.sh
[ -n "$(stamp_has "$(tree_key)" gate)" ] || { echo "gate_docs_test: the tree was not stamped as gated"; exit 1; }
echo "gate_docs_test: a documents-only change passes and is stamped"
