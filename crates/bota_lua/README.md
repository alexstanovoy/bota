# bota_lua

Lua bot API bindings over filtered `ClientSession` observations. Language execution,
collection, module-name validation and interpreter limits are provided by `luagee`.
The binding issues ordinary `IssuedOrder`s; it changes no simulation rules.

One `LuaTeam` owns a team's state. Each override has its own environment, while
`require` shares the team's module cache. Takeovers run every tick; otherwise modes
arbitrate every nine ticks with transition hooks. Team, ability/item, purchase and
courier hooks use the same scheduler. A failed bot turn discards that bot's orders.
Repeated fuel diagnostics are eligible once per thirty ticks.

Team push/defend hooks return three array entries in engine TOP, MID, BOT order:
finite values in [0, 1], with index 4 nil. The decoder normalizes negative zero.
The pair is published as soon as both lane hooks succeed, independently of errors
in TeamThink, farm, roam or Roshan hooks. If either lane hook fails or returns a
malformed value, the previously published pair is kept for that frame; an absent
or nil lane hook resets its own value to native policy when the pair succeeds.
A team-hook error discards only the first configured Lua seat's orders (index 0,
which need not be wire slot 0); other seats still run under the shared tick budget.
These are script decisions, so late output does not rewind published desires with
command continuations. Typed lane access translates core MID=0/TOP=1/BOT=2 into
the engine order rather than indexing the stored values with a core id.

For a preselected two-player Shadow Fiend Solo Mid match, missing mode desires (or a nil desire)
use the native fallback; missing mode Think hooks use its actions. Explicit empty Think functions
remain overrides. The current native slice covers laning, visible attacks, retreat, nearby farming,
legal skill/talent learning, raze/Requiem use, carried regeneration items, a baseline equipment
build and courier delivery. A full takeover receives no competing native fallback actions.
A hero file that only supplies MinionThink keeps hero modes and auxiliary defaults;
an explicit minion hook replaces native courier management, including an empty hook.
Mode sources are loaded only when needed and retain the active turn's fuel budget.
Policy thresholds and the equipment build are documented reconstruction assumptions;
this does not claim the closed engine's exact policy. Buyback and queued-action
completion are separate capabilities. Native work enters through hidden pinned
callbacks under the same tick fuel.

Defaults are 500,000 fuel per call, 2,000,000 per team tick and 256 MiB memory.
Host storage reserves 16 MiB against that cap, including bounded snapshot replacement
and handle maps. Snapshots hold at most 4 MiB of copied facts; unit and ability caches
hold at most 8,192 entries each; each bot returns at most 64 orders per turn.
Module banks validate names, module count and source sizes before copying, and
reserve storage for module-loading copies. Roots and userdata retention stay charged
through collection. All numeric arguments crossing into protocol coordinates are
finite; coordinate conversion uses nearest-even Q16.16 rounding and a 9,216 offset.

The default crate uses no filesystem or threads. Embedders of `LuaTeam`, including
Wasm hosts, must supply an execution deadline boundary. Feature `native` supplies
`NativeTeam`: a 16 MiB worker stack, one outstanding request, a 33 ms think deadline,
and retirement after three consecutive missed ticks or a host panic.
Late replies are discarded before submitting another snapshot; a timely reply
resets the miss counter. The next request acknowledges whether the previous
turn was accepted; discarded host continuations, including attack-once completion,
are restored before computing that request. Lua script globals are not rewound.
Native defaults are 125,000 fuel per call and 600,000
per team tick, leaving margin inside the deadline under load. Native interpreter limits may
only reduce the defaults. Active and retiring workers share a process-wide limit
of 32; cleanup does not wait on timed-out work.

Native directory preloading requires Linux `openat2` and procfs. It pins objects
before classification, rejects symlinks/submounts/nonregular files and reads sources
through pinned descriptors. Tree depth, entry count and source bytes are bounded.
Setup I/O has no latency guarantee. Thinks load exclusively from the completed bank.
`bots/` and bare module names address pack sources; `game/` addresses the optional
runtime-source root. Diagnostics use logical module names.

## Building and testing

`bota_lua` is a workspace of its own, outside the bota workspace: it builds on `luagee`, which must be checked out
beside the bota repository (`../luagee`, relative to the repository root), and the bota workspace builds without it.
Run these from the bota repository root; the build output goes to `crates/bota_lua/target`.

```sh
cargo build --release --manifest-path crates/bota_lua/Cargo.toml --features native      # the bota_botscript binary
cargo test --profile test-release --manifest-path crates/bota_lua/Cargo.toml --features native
cargo clippy --manifest-path crates/bota_lua/Cargo.toml --all-targets --all-features -- -D warnings
```

`tools/precheck.sh` and `tools/gate.sh` cover it as well (clippy, the wasm check, the fuzz target, `cargo deny`, fmt and
the `lua` test group) wherever the luagee checkout is present.

Run `bota_botscript` (`crates/bota_lua/target/release/bota_botscript`) with `--scripts DIR --team radiant`.
Options include `--runtime-scripts DIR`, `--address HOST:PORT`, `--hero KEY` and
`--think-ms N`. `--selection fixed` uses the configured hero without loading draft hooks;
the default `--selection pack` lets draft hooks choose and uses `--hero` as a fallback.
It connects directly using `bota_client_core`. `BOTA_SEATS` accepts
server-style `key_hex:team:name[:kind]` reservations; only the selected team's bot
seats are used. Without credentials it joins one unreserved seat. Keys never belong
on the command line.

`bota_botscript --api-catalogue` prints the installed global, unit, ability and library names as TSV without
loading sources or connecting. Save that output, then run `python3 -I tools/bot_api_census.py DIR --catalogue FILE`
to inventory a preselected Shadow Fiend's entry/module closure. `--runtime DIR` includes runtime modules;
`--hero KEY` chooses another short hero name and `--pack-selection` includes selection hooks. The JSON includes
missing names with source locations, presence coverage, dialect features, unresolved imports and native mode
hook dependencies. A function-name reference separates engine API candidates from unclassified script helpers;
`--api-reference FILE` can replace it. Shared branches and ambiguous receivers are conservatively included.
Presence coverage is not execution, signature or behavior coverage; static analysis does not prove complete
dynamic reachability. Unclassified calls and unresolved imports require separate validation. Sources are read locally
through bounded pinned descriptors; they are never copied into the repository or executed by the scanner.

The normal test group runs original small scripts through complete matches.
It reports local-source conformance separately as not run when no pack is configured;
setting `BOTA_LUA_LOCAL_PACK` also enables that contract in the group.
`tools/test_lua_local.sh` is explicitly optional: set `BOTA_LUA_LOCAL_PACK` and,
when required, `BOTA_LUA_RUNTIME_SCRIPTS`. It runs the ignored local-source match
without copying source into the repository. `fuzz/fuzz_targets/lua_binding.rs` (here,
a fuzz workspace of its own run with `fuzz/fuzz.sh`) drives Lua source, hostile arguments,
collection modes and varied game states.
