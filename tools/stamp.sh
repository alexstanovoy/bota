#!/usr/bin/env bash
# Stamps of what has been checked, kept per tree (sourced by precheck.sh and gate.sh; not run on its own).
#
# A stamp names a tree by the hash of its content with formatting left out: the files that are not Rust sources of the
# workspace crates as they are (the tracked and the not ignored new ones; a file marked skip-worktree counts as the commit
# has it), the Rust sources of the crates as `cargo fmt --all` writes them. The scripts add the selection to the name
# (tools/test_groups.py plan's `scope`: `full`, or what the change was compared with and what that selected). So a change
# that only formats keeps the stamp, and the same content in another checkout of this repository has the same stamp.
# Stamps live in the repository's common git directory, one file per tree and selection:
#   precheck ok <time>        tools/precheck.sh passed
#   gate ok <time>            tools/gate.sh passed what the change affects
#   gate-full ok <time>       tools/gate.sh passed in full (GATE_FULL=1)

tree_key() {
    local idx real_idx rs
    idx=$(mktemp) && rm -f "$idx"
    real_idx=$(git rev-parse --git-path index)
    {
        (
            export GIT_INDEX_FILE=$idx
            git read-tree HEAD
            git add -A -- . > /dev/null 2>&1
            # files this checkout keeps different from the commit on purpose (skip-worktree): the commit's version
            GIT_INDEX_FILE=$real_idx git ls-files -v | sed -n 's/^S //p' | while IFS= read -r f; do
                if h=$(git rev-parse -q --verify "HEAD:$f" 2> /dev/null); then
                    git update-index --add --cacheinfo "$(git ls-tree HEAD -- "$f" | cut -d' ' -f1),$h,$f"
                fi
            done
            # the Rust of the workspace crates as `cargo fmt --all` writes it below; the rest (fuzz/, the importer) as stored
            git ls-files -s | grep -vE $'\tcrates/.*\\.rs$'
        )
        cargo fmt --all -- --emit stdout 2> /dev/null
    } | sha256sum | cut -c1-32
    rm -f "$idx"
}

stamp_file() { # key
    local dir
    dir=$(git rev-parse --git-common-dir)/checked-trees
    mkdir -p "$dir"
    echo "$dir/$1"
}

stamp_has() { # key what: the stamp line, empty when there is none
    grep -m1 "^$2 ok " "$(stamp_file "$1")" 2> /dev/null
}

stamp_put() { # key what
    echo "$2 ok $(date +%FT%T)" >> "$(stamp_file "$1")"
}

only_docs_changed() { # base: success when something changed since the merge-base with base and it is documents only
    local mb files
    mb=$(git merge-base HEAD "$1" 2> /dev/null) || return 1
    files=$({ git diff --name-only "$mb"; git ls-files --others --exclude-standard; } | sort -u)
    [ -n "$files" ] && ! grep -qvE '\.md$|^docs/|^temp/|^LICENSE' <<< "$files"
}

default_base() { # what a change is measured against: origin/main, or main where there is no origin
    if git rev-parse -q --verify origin/main > /dev/null; then echo origin/main; else echo main; fi
}
