#!/usr/bin/env bash
# The builds of the wires this build admits (bota_core::ADMITTED_WIRES): their corpora, and their
# server and peer for the cross-build tests (crates/bota_server/tests/admitted_builds.rs).
#
#   tools/admitted.sh corpus COMMIT   builds COMMIT with this tree's corpus program and writes the
#                                     corpus of its wire to crates/bota_server/tests/admitted/WIRE/
#   tools/admitted.sh build           builds the server and the peer of every corpus' commit (its
#                                     corpus.txt names it)
#
# The cache is <Cargo's target directory>/admitted; ADMITTED_BUILDS (an absolute path) names
# another. A commit is exported with `git archive` (never a checkout of this tree), this tree's
# programs (examples/admitted_corpus.rs, examples/admitted_peer.rs, tests/admitted/scenarios.rs and
# tests/admitted/peer.rs) are laid over it, and it is built in release with its own lock file in a
# work directory. Only after the build succeeded and its server said the expected wire are the
# executables kept, as WIRE/bin/ with WIRE/wire, WIRE/sums (the executables' SHA-256) and
# WIRE/stamp (the commit and the programs they were built from, written last); the work
# directory is then deleted, after a failure too.
#
# Nothing is trusted unchecked: a cached entry is served only when it is a real directory whose
# stamp matches, whose executables are regular files matching their sums, and whose server says
# the wire recorded; any other entry is deleted and built again, so nothing of an earlier build is
# run after a failed rebuild. The cache directory may not be a symbolic link; runs on one cache
# take turns by a lock on the directory itself (no lock file is written or followed). Every
# failure exits non-zero. Retention: `build` deletes the entries of wires no corpus names, in the
# default cache only (an ADMITTED_BUILDS directory may be another worktree's); the cache must stay
# under ADMITTED_MAX_BYTES (a whole number of bytes, 1 GiB by default), or the script fails. See
# DEVELOPMENT.md, "Admitted wires".
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
corpora=$root/crates/bota_server/tests/admitted
programs=(
    crates/bota_server/examples/admitted_corpus.rs
    crates/bota_server/examples/admitted_peer.rs
    crates/bota_server/tests/admitted/scenarios.rs
    crates/bota_server/tests/admitted/peer.rs
)
executables=(bota_server examples/admitted_corpus examples/admitted_peer)
# The work directory of the build under way, deleted when the script fails.
work=

fail() {
    echo "tools/admitted.sh: $*" >&2
    if [ -n "$work" ]; then
        rm -rf -- "$work"
    fi
    exit 1
}

# Settings, checked before any work.
budget=${ADMITTED_MAX_BYTES-1073741824}
# Up to 18 digits: bash's integers hold them.
[[ $budget =~ ^[1-9][0-9]{0,17}$ ]] ||
    fail "ADMITTED_MAX_BYTES must be a whole number of bytes from 1 to 18 digits, not ${budget@Q}"
if [ -n "${ADMITTED_BUILDS+set}" ]; then
    cache=$ADMITTED_BUILDS
    [[ $cache == /* ]] || fail "ADMITTED_BUILDS must be an absolute path, not ${cache@Q}"
    own_cache=false
else
    target=$(cargo metadata --format-version 1 --no-deps --manifest-path "$root/Cargo.toml" |
        sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p') ||
        fail "cannot ask Cargo for its target directory"
    [[ $target == /* ]] || fail "Cargo named no target directory"
    cache=$target/admitted
    own_cache=true
fi
cache=${cache%/}
[ -L "$cache" ] && fail "the cache $cache is a symbolic link"
mkdir -p -- "$cache" || fail "cannot make the cache $cache"
[ -d "$cache" ] && [ ! -L "$cache" ] || fail "the cache $cache is not a directory"
# One run at a time on a cache: a lock on the directory, opened for reading.
exec 9<"$cache" || fail "cannot open $cache"
flock 9 || fail "cannot lock $cache"

# What a build of COMMIT with this tree's programs is stamped with.
stamp_of() {
    { echo "$1"; cd -- "$root" && sha256sum -- "${programs[@]}"; } | sha256sum
}

# Whether DIR holds a build stamped STAMP that may be served: a real directory, regular files
# untouched since they were kept, and a server saying the wire recorded. Never fails the script.
served() {
    local dir=$1 stamp=$2 wire said
    [ -d "$dir" ] && [ ! -L "$dir" ] || return 1
    for file in stamp wire sums; do
        [ -f "$dir/$file" ] && [ ! -L "$dir/$file" ] || return 1
    done
    [ "$(cat -- "$dir/stamp")" = "$stamp" ] || return 1
    for executable in "${executables[@]}"; do
        [ -f "$dir/bin/$executable" ] && [ ! -L "$dir/bin/$executable" ] &&
            [ -x "$dir/bin/$executable" ] || return 1
    done
    (cd -- "$dir/bin" && sha256sum --quiet --strict -c -- ../sums) >/dev/null 2>&1 || return 1
    wire=$(cat -- "$dir/wire")
    said=$("$dir/bin/bota_server" --wire-fingerprint 2>/dev/null) || return 1
    [ "$said" = "$wire" ]
}

# Builds COMMIT into DIR (DIR/bin, DIR/wire, DIR/sums, DIR/stamp), unless DIR holds a build of it
# with these programs that may be served. Fails, leaving no DIR and no work directory, when
# anything of it fails.
build_commit() {
    local commit=$1 dir=$2 stamp wire
    stamp=$(stamp_of "$commit") || fail "cannot read the programs to lay over $commit"
    if served "$dir" "$stamp"; then
        return 0
    fi
    rm -rf -- "$dir" || fail "cannot delete the stale build $dir"
    work=$dir.work
    rm -rf -- "$work" || fail "cannot delete $work"
    mkdir -p -- "$work/src" || fail "cannot make $work"
    git -C "$root" archive --output="$work/src.tar" "$commit" || fail "cannot export $commit"
    tar -x -C "$work/src" -f "$work/src.tar" || fail "cannot unpack the export of $commit"
    for program in "${programs[@]}"; do
        mkdir -p -- "$(dirname -- "$work/src/$program")" || fail "cannot lay $program over $commit"
        cp -- "$root/$program" "$work/src/$program" || fail "cannot lay $program over $commit"
    done
    # bota_radovid's manifest names the radovid checkout beside the repository; every member's
    # manifest is read even for a build of one crate, so the export sees the same checkout.
    if [ -d "$root/../radovid" ]; then
        ln -sfn -- "$(cd -- "$root/../radovid" && pwd)" "$work/radovid" || fail "cannot link radovid"
    fi
    CARGO_TARGET_DIR=$work/target cargo build --release --locked -q \
        --manifest-path "$work/src/Cargo.toml" -p bota_server \
        --bin bota_server --example admitted_corpus --example admitted_peer >&2 ||
        fail "building $commit failed"
    mkdir -p -- "$work/bin/examples" || fail "cannot make $work/bin"
    for executable in "${executables[@]}"; do
        cp -- "$work/target/release/$executable" "$work/bin/$executable" ||
            fail "the build of $commit made no $executable"
    done
    wire=$("$work/bin/bota_server" --wire-fingerprint) || fail "the server of $commit says no wire"
    [[ $wire =~ ^[0-9a-f]{16}$ ]] || fail "the server of $commit says the wire ${wire@Q}"
    (cd -- "$work/bin" && sha256sum -- "${executables[@]}") >"$work/sums" ||
        fail "cannot sum the build of $commit"
    mkdir -- "$dir" || fail "cannot make $dir"
    mv -- "$work/bin" "$dir/bin" || fail "cannot keep the build of $commit"
    mv -- "$work/sums" "$dir/sums" || fail "cannot keep the sums of $commit"
    echo "$wire" >"$dir/wire" || fail "cannot keep the wire of $commit"
    echo "$stamp" >"$dir/stamp" || fail "cannot stamp the build of $commit"
    rm -rf -- "$work" || fail "cannot delete $work"
    work=
}

# Fails when the cache is over its budget.
within_budget() {
    local used
    used=$(du -sb -- "$cache" | cut -f1) || fail "cannot measure $cache"
    [[ $used =~ ^[0-9]+$ ]] || fail "cannot measure $cache: ${used@Q}"
    if [ "$used" -gt "$budget" ]; then
        fail "the admitted builds take $used bytes in $cache, over ADMITTED_MAX_BYTES=$budget: \
delete the entries of wires no longer admitted, or raise the budget"
    fi
}

case "${1:-}" in
    corpus)
        [ $# -eq 2 ] || { echo "usage: tools/admitted.sh corpus COMMIT" >&2; exit 2; }
        commit=$(git -C "$root" rev-parse --verify --end-of-options "$2^{commit}") ||
            fail "no commit $2"
        scratch=$cache/commit-$commit
        build_commit "$commit" "$scratch"
        wire=$(cat -- "$scratch/wire") || fail "no wire for $commit"
        mkdir -p -- "$corpora/$wire" || fail "cannot make the corpus directory of $wire"
        "$scratch/bin/examples/admitted_corpus" "$corpora/$wire" "$commit" ||
            fail "the corpus program of $commit failed"
        rm -rf -- "${cache:?}/$wire" || fail "cannot replace the cached build of $wire"
        mv -- "$scratch" "$cache/$wire" || fail "cannot keep the build of $wire"
        within_budget
        echo "corpus of wire $wire written to crates/bota_server/tests/admitted/$wire" >&2
        ;;
    build)
        [ $# -eq 1 ] || { echo "usage: tools/admitted.sh build" >&2; exit 2; }
        named=()
        for manifest in "$corpora"/*/corpus.txt; do
            [ -e "$manifest" ] || continue
            wire=$(basename -- "$(dirname -- "$manifest")")
            [[ $wire =~ ^[0-9a-f]{16}$ ]] || fail "the corpus $manifest is not named by a wire"
            named+=("$wire")
            commit=$(sed -n 's/^commit //p' "$manifest")
            [[ $commit =~ ^[0-9a-f]{40}$ ]] || fail "$manifest names no commit (40 hex digits)"
            build_commit "$commit" "$cache/$wire"
            built=$(cat -- "$cache/$wire/wire") || fail "no wire for $commit"
            if [ "$built" != "$wire" ]; then
                rm -rf -- "${cache:?}/$wire"
                fail "commit $commit builds wire $built, its corpus is of $wire"
            fi
        done
        if $own_cache; then
            for entry in "$cache"/*; do
                [ -e "$entry" ] || [ -L "$entry" ] || continue
                name=$(basename -- "$entry")
                keep=false
                for wire in "${named[@]}"; do
                    [ "$name" = "$wire" ] && keep=true
                done
                $keep || rm -rf -- "$entry" || fail "cannot delete $entry"
            done
        fi
        within_budget
        ;;
    *)
        echo "usage: tools/admitted.sh corpus COMMIT | build" >&2
        exit 2
        ;;
esac
