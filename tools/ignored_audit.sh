#!/usr/bin/env bash
# Every #[ignore]d test of this repository must be run by a step of the gate (or be a recorder, which asserts nothing and
# is run by hand to write pins): a test nobody runs is a test nobody notices failing. Fails and names each file that holds an
# ignored test no step claims. A new ignored test needs its step, or a line here saying which step runs it.
#
#   tools/ignored_audit.sh
set -euo pipefail
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
cd "$here"
bad=0
while IFS= read -r file; do
    case "$file" in
        tools/import_items/src/main.rs) ;;                     # gate step import: cargo test -- --include-ignored
        crates/bota_server/src/game/tests/determinism.rs) ;;   # print_pins, print_blind_pins: recorders (cargo test print_pins -- --ignored --nocapture)
        *) echo "ignored_audit: $file has ignored tests that no gate step runs"; bad=1 ;;
    esac
done < <(git ls-files '*.rs' | xargs grep -l '#\[ignore')
[ "$bad" = 0 ] && echo "ignored_audit: every ignored test has a step"
exit "$bad"
