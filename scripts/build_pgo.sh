#!/usr/bin/env bash
# Profile-guided release build of bota_server (the server binary and the `dummy` benchmark), reproducible from a commit.
#
#   scripts/build_pgo.sh build  [COMMIT]   # -> $PGO_WORK/out/{bota_server,dummy-bench} (PGO) and *-base (PGO_BASELINE=1)
#   scripts/build_pgo.sh verify            # determinism pins and the benchmark digests under the PGO build
#
# Builds an instrumented benchmark, runs the tick loop, the Map2 skirmish, the micro cases and the local match example
# as training workloads, merges the profiles with llvm-profdata (rustup component add llvm-tools) and rebuilds with
# -Cprofile-use. PGO changes only code layout and inlining; simulation results stay bit-identical.
# Wrap it in your resource wrapper if you have one: HEAVY="path/to/wrapper" (default: run directly).
# Environment: PGO_WORK (default <repo>/target/pgo), PGO_BASELINE=1 (also build the normal binaries).
# RUSTFLAGS replaces cargo config rustflags, so BASE_FLAGS (default "-C target-cpu=native") is repeated here.
# The merged profile is named by its content hash so a compiler cache can never serve a stale profile build.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
work=${PGO_WORK:-$repo/target/pgo}
base_flags=${BASE_FLAGS:--C target-cpu=native}
read -r -a heavy <<<"${HEAVY:-}"
cmd=${1:?usage: build_pgo.sh build|verify}
shift

llvm_profdata() {
    echo "$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/llvm-profdata"
}

# Path of the benchmark executable cargo builds with the current RUSTFLAGS and target dir.
bench_exe() {
    "${heavy[@]}" cargo bench -p bota_server --bench dummy --no-run 2>&1 | sed -n 's/.*(\(.*\/deps\/dummy-[0-9a-f]*\)).*/\1/p' | tail -1
}

build() {
    local commit=${1:-HEAD}
    commit=$(git -C "$repo" rev-parse "$commit")
    mkdir -p "$work/src" "$work/out" "$work/prof"
    local src=$work/src/bota
    if [ -e "$src/.git" ]; then git -C "$src" checkout --quiet --force --detach "$commit"
    else git -C "$repo" worktree add --detach "$src" "$commit" >/dev/null; fi
    cd "$src"

    echo "== instrumented build"
    export CARGO_TARGET_DIR=$work/target-gen RUSTFLAGS="$base_flags -Cprofile-generate=$work/prof"
    "${heavy[@]}" cargo build --release -p bota_server --example local_match
    local gen
    gen=$(bench_exe)
    echo "== training workloads"
    find "$work/prof" -name '*.profraw' -delete
    export LLVM_PROFILE_FILE="$work/prof/%m-%p.profraw"
    "${heavy[@]}" "$(realpath "$gen")" --bench --profile-time 6 'tick_loop'
    "${heavy[@]}" "$(realpath "$gen")" --bench --profile-time 12 'match/map2_skirmish'
    "${heavy[@]}" "$(realpath "$gen")" --bench --profile-time 4 'micro'
    "${heavy[@]}" "$CARGO_TARGET_DIR/release/examples/local_match" >/dev/null
    unset LLVM_PROFILE_FILE

    echo "== merge"
    "${heavy[@]}" "$(llvm_profdata)" merge -o "$work/merged.profdata" "$work"/prof/*.profraw
    local hash
    hash=$(sha256sum "$work/merged.profdata" | cut -c1-12)
    cp "$work/merged.profdata" "$work/out/bota-$hash.profdata"

    echo "== PGO build"
    export CARGO_TARGET_DIR=$work/target-use RUSTFLAGS="$base_flags -Cprofile-use=$work/out/bota-$hash.profdata -Cllvm-args=-pgo-warn-missing-function=false"
    "${heavy[@]}" cargo build --release -p bota_server --bin bota_server
    cp "$CARGO_TARGET_DIR/release/bota_server" "$work/out/bota_server"
    cp "$(realpath "$(bench_exe)")" "$work/out/dummy-bench"
    if [ "${PGO_BASELINE:-0}" = 1 ]; then
        echo "== baseline build"
        export CARGO_TARGET_DIR=$work/target-base RUSTFLAGS="$base_flags"
        "${heavy[@]}" cargo build --release -p bota_server --bin bota_server
        cp "$CARGO_TARGET_DIR/release/bota_server" "$work/out/bota_server-base"
        cp "$(realpath "$(bench_exe)")" "$work/out/dummy-bench-base"
    fi
    echo "built $work/out (commit $commit, profile $hash)"
}

# The pins run as tests compiled with the PGO profile; the benchmark asserts its own digests before it measures.
verify() {
    local profile
    profile=$(ls -t "$work"/out/bota-*.profdata | head -1)
    cd "$work/src/bota"
    export CARGO_TARGET_DIR=$work/target-use RUSTFLAGS="$base_flags -Cprofile-use=$profile -Cllvm-args=-pgo-warn-missing-function=false"
    "${heavy[@]}" cargo test --release -p bota_server
    "${heavy[@]}" "$work/out/dummy-bench" --bench --profile-time 1 'tick_loop|map2_skirmish|plan'
    echo IDENTICAL
}

case $cmd in
    build) build "$@" ;;
    verify) verify ;;
    *) echo "unknown command $cmd" >&2; exit 2 ;;
esac
