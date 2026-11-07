#!/usr/bin/env bash
# The build checks a change needs, by the crates it touches (sourced by gate.sh and precheck.sh; not run on its own).
# `lint_steps` and `wasm_steps` take the plan's crates (the changed ones and every crate building on them) and `fuzz_steps`
# the plan's fuzz flag; each prints one line per step, tab separated: its name, 1 when the change needs it and 0 otherwise,
# why it is not needed, its command (words without quoting). The gate names every step and runs the needed ones; the
# precheck runs the needed ones.
# bota_lua (the bot-script host) is a workspace of its own that needs the luagee checkout beside this repository: it is in the
# plan's crates only where that checkout is, and its steps use its manifest.
botscript=crates/bota_lua/Cargo.toml

has() { # list word: success when word is in the space separated list
    case " $1 " in *" $2 "*) return 0 ;; esac
    return 1
}

needed() { # list word: 1 when word is in the list, 0 otherwise
    if has "$1" "$2"; then echo 1; else echo 0; fi
}

lint_steps() { # crates
    local crates=$1 packages="" crate unchanged="and what it builds on are unchanged"
    for crate in $crates; do
        case "$crate" in bota_radovid | bota_lua) ;; *) packages+=" -p $crate" ;; esac
    done
    printf '%s\t%s\t%s\t%s\n' \
        clippy "$([ -n "$packages" ] && echo 1 || echo 0)" "no crate changed" \
        "cargo clippy$packages --all-targets -- -D warnings" \
        clippy-radovid "$(needed "$crates" bota_radovid)" "bota_radovid $unchanged" \
        "cargo clippy -p bota_radovid --all-targets --features e2e -- -D warnings" \
        clippy-lua "$(needed "$crates" bota_lua)" "bota_lua $unchanged" \
        "cargo clippy --manifest-path $botscript --all-targets --all-features -- -D warnings" \
        fuzz-lua "$(needed "$crates" bota_lua)" "bota_lua $unchanged" \
        "crates/bota_lua/fuzz/fuzz.sh check" \
        clippy-ui "$(needed "$crates" bota_client_core)" "bota_client_core $unchanged" \
        "cargo clippy -p bota_client_core --all-targets --features ui -- -D warnings"
}

fuzz_steps() { # the plan's fuzz flag (0 or 1): the fuzz targets compile, nothing is fuzzed
    printf '%s\t%s\t%s\t%s\n' fuzz-check "$1" "no crate the fuzz targets build on and no fuzz target changed" "fuzz/fuzz.sh check"
}

wasm_steps() { # crates
    local crates=$1 unchanged="and what it builds on are unchanged"
    printf '%s\t%s\t%s\t%s\n' \
        wasm "$(needed "$crates" bota_client_core)" "bota_client_core $unchanged" \
        "cargo check --target wasm32-unknown-unknown -p bota_client_core -p bota_proto --features bota_client_core/ui" \
        wasm-lua "$(needed "$crates" bota_lua)" "bota_lua $unchanged" \
        "cargo check --release --target wasm32-unknown-unknown --manifest-path $botscript --no-default-features" \
        wasm-radovid "$(needed "$crates" bota_radovid)" "bota_radovid $unchanged" \
        "cargo check --target wasm32-unknown-unknown -p bota_radovid --features web" \
        wasm-web "$(needed "$crates" bota_client_core)" "bota_client_core $unchanged" \
        "cargo check --target wasm32-unknown-unknown -p bota_client_core --features bota_client_core/web"
}
