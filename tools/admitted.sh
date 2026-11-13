#!/usr/bin/env bash
# The builds of the wires this build admits (bota_core::ADMITTED_WIRES): their corpora.
#
#   tools/admitted.sh corpus COMMIT   builds COMMIT with this tree's corpus program and writes the
#                                     corpus of its wire to crates/bota_server/tests/admitted/WIRE/
#
# ADMITTED_BUILDS names another directory than target/admitted. A commit is exported with
# `git archive` (never a checkout of this tree) to target/admitted/WIRE/src,
# this tree's programs (examples/admitted_corpus.rs and tests/admitted/scenarios.rs) are laid over it, and it is built in release with its own lock file
# and a target directory of its own beside the export. See DEVELOPMENT.md, "Admitted wires".
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
target=${ADMITTED_BUILDS:-${CARGO_TARGET_DIR:-$root/target}/admitted}
corpora=$root/crates/bota_server/tests/admitted
programs=(
    crates/bota_server/examples/admitted_corpus.rs
    crates/bota_server/tests/admitted/scenarios.rs
)

# Exports COMMIT to DIR/src with this tree's programs over it and builds it in DIR/target; prints
# the wire of the build.
build_commit() {
    local commit=$1 dir=$2 stamp
    # An export already made of this commit and these programs is kept, so Cargo builds nothing
    # again.
    stamp=$(cd -- "$root" && { echo "$commit"; sha256sum -- "${programs[@]}"; } | sha256sum)
    if [ "$(cat -- "$dir/exported" 2>/dev/null)" != "$stamp" ]; then
        rm -rf -- "$dir/src" "$dir/exported"
        mkdir -p -- "$dir/src"
        git -C "$root" archive "$commit" | tar -x -C "$dir/src"
        for program in "${programs[@]}"; do
            mkdir -p -- "$(dirname -- "$dir/src/$program")"
            cp -- "$root/$program" "$dir/src/$program"
        done
        echo "$stamp" >"$dir/exported"
    fi
    # bota_radovid's manifest names the radovid checkout beside the repository; every member's
    # manifest is read even for a build of one crate, so the export sees the same checkout.
    ln -sfn -- "$(cd -- "$root/../radovid" && pwd)" "$dir/radovid"
    CARGO_TARGET_DIR=$dir/target cargo build --release --locked -q \
        --manifest-path "$dir/src/Cargo.toml" -p bota_server \
        --bin bota_server --example admitted_corpus >&2
    "$dir/target/release/bota_server" --wire-fingerprint
}

case "${1:-}" in
    corpus)
        [ $# -eq 2 ] || { echo "usage: tools/admitted.sh corpus COMMIT" >&2; exit 2; }
        commit=$(git -C "$root" rev-parse --verify "$2^{commit}")
        scratch=$target/commit-$commit
        wire=$(build_commit "$commit" "$scratch")
        mkdir -p -- "$corpora/$wire"
        "$scratch/target/release/examples/admitted_corpus" "$corpora/$wire" "$commit"
        rm -rf -- "$target/$wire"
        mv -- "$scratch" "$target/$wire"
        echo "corpus of wire $wire written to crates/bota_server/tests/admitted/$wire" >&2
        ;;
    *)
        echo "usage: tools/admitted.sh corpus COMMIT" >&2
        exit 2
        ;;
esac
