#!/usr/bin/env bash
# Every #[ignore]d test needs a named gate step or a documented reason to run it by hand.
# Fixture recorders may require an older source tree or a separate setup.
# List each manual recorder by test name in MANUAL_RECORDERS in tools/test_groups.py.
#
#   tools/ignored_audit.sh
set -euo pipefail
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
cd "$here"
bad=0
while IFS= read -r file; do
    case "$file" in
        crates/bota_lua/tests/local_scripts.rs) ;;             # explicit optional tools/test_lua_local.sh, gated by BOTA_LUA_LOCAL_PACK
        tools/import_items/src/main.rs|tools/import_items/src/wave_rules.rs) ;; # group import_items (tools/test_groups.toml): --include-ignored
        tools/import_map/src/main.rs) ;;                       # group import_map (tools/test_groups.toml): --include-ignored
        crates/bota_server/src/game/tests/determinism.rs) ;;   # print_pins, print_blind_pins: recorders (cargo test print_pins -- --ignored --nocapture)
        tools/import_items/src/neutral.rs|tools/import_items/src/boss.rs) ;;                # group import_items (tools/test_groups.toml): --include-ignored
        crates/bota_server/src/game/tests/fixtures/creep_bounty_fixture.rs)
            # Recorder requires the 025607d6 rules. Install its test module in that source archive as its header describes.
            # From the extracted tree, with its tarball saved as base.tar:
            # BOTA_BOUNTY_FIXTURE_BASE="$(git get-tar-commit-id < base.tar)" cargo test \
            #   --profile test-release --locked -p bota_server --lib capture_creep_bounty_fixture -- --ignored --nocapture \
            #   | sed -n 's/^FIXTURE //p' > creep_bounties_025607d6.txt
            ;;
        *) echo "ignored_audit: $file has ignored tests that no gate step runs"; bad=1 ;;
    esac
done < <(git ls-files '*.rs' | xargs grep -l '#\[ignore')
[ "$bad" = 0 ] && echo "ignored_audit: every ignored test is classified"
exit "$bad"
