# bota_lua

Lua bot API bindings over filtered `ClientSession` observations. Language execution,
collection, module-name validation and interpreter limits are provided by `luagee`.
The binding issues ordinary `IssuedOrder`s; it changes no simulation rules.

One `LuaTeam` owns a team's state. Each override has its own environment, while
`require` shares the team's module cache. Takeovers run every tick; otherwise modes
arbitrate every nine ticks with transition hooks. Team, ability/item, purchase and
courier hooks use the same scheduler. A failed bot turn discards that bot's orders.
Repeated fuel diagnostics are eligible once per thirty ticks.

Team push/defend hooks return a table containing exactly numeric keys 1, 2 and 3
in engine TOP, MID, BOT order, with finite values in [0, 1]. Sparse trailing keys
and other extra fields are rejected by a bounded raw traversal. The decoder normalizes negative zero.
The pair is published as soon as both lane hooks succeed, independently of errors
in TeamThink, farm, roam or Roshan hooks. If either lane hook fails or returns a
malformed value, the previously published pair is kept for that frame; an absent
or nil lane hook resets its own value to native policy when the pair succeeds.
Every team hook is attempted independently, including farm, roam and Roshan after
an earlier error. The first error is reported; later errors increment
`HookActivity::suppressed_team_errors`, included in warnings and match totals.
A team-hook error discards only
the first configured Lua seat's orders (index 0,
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

## Native reconstruction evidence

“Sourced” below identifies a fact read from the current simulation catalogue or
received observations. It does not independently verify that catalogue against
Dota. “Inferred” is a compatibility interpretation awaiting an engine check.
“Own design” is an explicit host policy, not a recovered Valve C++ decision.
Tests pin these choices; passing a pin does not promote an inference to a source.

| Choice | Classification | Evidence or reason |
|---|---|---|
| Two SF heroes, MAP2 only | Own design | Scope of the native fallback; other configurations receive no native policy. |
| Mode arbitration every 9 ticks | Own design | Bounds repeated arbitration work; overrides still Think each tick. |
| Laning/attack/farm/retreat desires 0.3/0.6/0.4/0.95 | Own design | Explicit ordering of the reconstructed modes. No engine-policy source. |
| Retreat below 35% HP, held until 90% | Own design | Hysteresis avoids repeatedly switching modes near a threshold. |
| Attack/farm radius 900; item/courier threat radius 1000 | Own design | Local decision radii, not claimed engine policy. |
| Raze distances, radii, damage and mana costs | Sourced: simulation catalogue | `bota_core::rules::RAZE_DISTANCE`, `RAZE_RADIUS`, hero definitions and received ability rows. |
| Raze creep cluster at least 2; mana reserve at least twice the cost | Own design | Reconstructed resource and target selection. |
| Requiem requires received souls and an enemy within attack range | Own design using sourced facts | The decision is ours; souls, range and ability metadata are received/catalogue facts. |
| Learn available ultimate before ordinary active skills, then passives, by slot; available talents first | Own design using sourced legality | Selection priority is ours; `learnable` and `may_learn` come from the view. |
| Regeneration useful at half its restoration amount; avoid interruptible recovery near threats; Phase during retreat | Own design using sourced effects | Amounts, aim, active effect, mute and cooldown are read from item/effect metadata. |
| Equipment: 2 branches, Wraith Band, Bottle, Boots, Wand, Treads, Dragon Lance | Own design | Baseline equipment goals, not an engine build. Recipes and purchase legality are catalogue/binding data. |
| Recipe projection: 1024 visits, depth 16, bounded pending stack | Own design | Work ceiling. Exhaustion skips the entire optional equipment projection; VM fuel exhaustion still errors the turn. |
| Courier command lease 30 ticks | Own design | Retry lost/rejected requests without resending every tick. Tick rate is sourced from rules. |
| Courier delivery/stash/return, burst and shield selection | Own design using sourced legality | Ability ids, cooldown, mana, ownership and handoff/stash ranges come from catalogue/views. |
| Lane target 200 units behind the observed friendly front, quantized to 256-unit cells towards home | Own design | Stable waypoints prevent repeated tiny replacements of standing movement. |
| Native attack-once lease: observed windup plus 1 second, clamped to 1–10 seconds | Own design using received timing | Bounds silence after an unlaunched order is rejected or lost. Launch facts take precedence at expiry. Explicit script once requests have no native lease. Discarded host turns restore the previous continuation checkpoint. |
| Missing/nil GetDesire falls back; explicit empty Think overrides; MinionThink-only files retain hero defaults; explicit minion hooks replace courier defaults | Inferred compatibility contract | Implements the host's documented composition of scripts and native policy. Not verified against the closed engine. |
| Engine player ids: Radiant 0–4, Dire 5–9; rank from the received roster, only a proven contiguous prefix before that roster | Inferred compatibility contract | Distinct from wire seat ids. No live-engine mapping verification is claimed. |
| BOT_MODE ids 1–22 in `team::BOT_MODES` order: laning, attack, roam, retreat, secret_shop, side_shop, push_top/mid/bot, defend_top/mid/bot, assemble, team_roam, farm, defend_ally, evasive_maneuvers, roshan, item, ward, rune, outpost | Inferred compatibility contract | Host constants pinned to this order; original engine constants remain to be verified. |
| Engine lane TOP=1/MID=2/BOT=3 mapped to core MID=0/TOP=1/BOT=2 | Inferred engine ids; sourced core ids | Typed conversion pins the distinction; the pin is not engine evidence. |
| GetAttackPoint base seconds; GetAttackSpeed normalized multiplier | Inferred compatibility contract | Documented in API.md; conflicting script usage requires live-engine verification. |

The inferred contracts and unsourced policy choices require verification or an
explicit decision to retain an own-design policy before claiming engine parity.
