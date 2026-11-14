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
this does not claim the closed engine's exact policy. The native fallback never buys
back (scripts may, through the buyback calls); queued-action completion is a separate
capability. Native work enters through hidden pinned
callbacks under the same tick fuel.

The first source-backed Laning slice replaces pregame attack-movement with a
controller-owned move 300 units behind the observed own mid T1, for the no-creep
single-bot case. Its 100-unit arrival rule, reference build,
transport adaptation and unimplemented branches are documented in [native_laning.md](native_laning.md).

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
only reduce the defaults of the script host: `SCRIPT_LIMITS`, the interpreter's defaults with the upvalues per function
raised to 120, which the reference game's script runtime accepts and scripts written for it rely on (game build 6975).
`NativeConfig` and so `bota_botscript` start from it. `LuaTeam::new` takes the limits its caller passes, so an embedder that
passes the interpreter's `Limits::DEFAULT` keeps 60 upvalues per function. Active and retiring workers share a process-wide limit
of 32; cleanup does not wait on timed-out work.

Native directory preloading requires Linux `openat2` and procfs. It pins objects
before classification, rejects symlinks/submounts/nonregular files and reads sources
through pinned descriptors. Tree depth, entry count and source bytes are bounded.
Setup I/O has no latency guarantee. Thinks load exclusively from the completed bank.
`bots/` and bare module names address pack sources; `game/` addresses the optional
runtime-source root. Diagnostics use logical module names.

## Determinism

By default two runs of the same match can differ. `bota_botscript` draws the interpreter's hash key and the random
seed of `math.random` from the operating system's entropy at startup and prints them (`script_keys ...`) before it
connects. Every bot turn is bounded by fuel and by a wall-clock deadline (`--think-ms`, 33 by default; a turn that
misses it is discarded). Why: scripts are untrusted, so the hash key must be secret from them and differ per
interpreter, and play is live, so a slow script must not stall the match. Consequences: a script's `math.random`
draws differ between runs, and under machine load a turn can be discarded at a different point (`deadline_ticks`).

Reproducible mode needs both `--hash-keys A,B` and `--rng-seed N`. Supplying only one changes nothing about the
deadline: the other value is still drawn from entropy and the wall deadline stays. With both, the host runs
fuel-bounded with no wall deadline (`--think-ms` has no effect; a script that never ends is stopped by fuel, with
`deadline_ticks` 0), so the outcome of a turn depends on the script and its observations only. Two runs give the same
orders when all of these are equal: the pack and runtime sources, the `bota_botscript` binary (with the `luagee`
version it was built against), the keys and seed, the selection policy (`--selection`, `--hero`, `--team`) and the
game the bot is shown (the match seed and what the other seats order). Pinned by
`explicit_keys_reproduce_live_orders_and_two_offline_refeeds` and `measured_native_turn_reports_exhaustion_on_its_exact_tick`
in `tests/embedding_statistics.rs`, which play a local match and compare the orders of a live run with two re-feeds.

What each flag is for. The seed is the one that changes what a script sees: `math.random` follows it, and other seeds give
other orders (`orders_follow_the_random_seed_and_ignore_the_hash_key`). The hash key does not: the interpreter's output,
iteration order, fuel and memory accounting are identical under every key, and the same test shows equal orders under
another key. The host still asks for both so that a run states every input of its interpreter (both are printed and
written to the summary); a reproduction may use any hash key.
A bot match is reproducible only when the script's orders are: the orders of a script bot are an input to the match,
and the match itself stays deterministic given them.

`--record-views PATH` records the first joined seat's filtered observations. `--replay-in PATH --seat N --hash-keys A,B
--rng-seed N` re-feeds them to a fresh host synchronously, with no server, network or deadline, and recomputes the orders
the script gives for each recorded turn: the same orders as the live run that recorded them when that run was
reproducible and every item above is equal. The re-feed does not play the match (the recorded observations already
contain the effect of the orders given back then), it refuses fogless recordings and recordings of another wire
fingerprint. Not covered: a default-mode run, whose recording holds what that run saw but whose draws and discarded turns
cannot be repeated, and any change of pack, binary or interpreter version.

## Building and testing

`bota_lua` is a member of the bota workspace, outside the default members like `bota_radovid` and `bota_gym` (a plain
`cargo build` does not compile the interpreter), and builds on `luagee`, the Lua interpreter, from crates.io. Run these from
the bota repository root:

```sh
cargo build --release -p bota_lua --features native      # the bota_botscript binary
cargo test --profile test-release -p bota_lua --features native
cargo clippy -p bota_lua --all-targets --all-features -- -D warnings
```

`tools/precheck.sh` and `tools/gate.sh` cover it as well (clippy, the wasm check, the fuzz target and the `lua` test group).

Run `bota_botscript` (`target/release/bota_botscript`) with `--scripts DIR --team radiant`.
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
| Pregame movement command lease 30 ticks | Host execution contract | Initial command, bounded retry, immediate retarget and one cancellation on arrival. Shares the courier retry interval. |
| Courier delivery/stash/return, burst and shield selection | Own design using sourced legality | Ability ids, cooldown, mana, ownership and handoff/stash ranges come from catalogue/views. |
| Pregame no-creep Laning: move 300 behind own mid T1, 100-unit arrival tolerance | Established from the installed game, build 6975; host map/transport adaptation | See native_laning.md for behaviour, supported conditions and tests. |
| Other lane targets: 200 behind the observed friendly front, 256-unit cells towards home | Own design | Existing fallback outside the recovered pregame slice. |
| Native attack-once lease: observed windup plus 1 second, clamped to 1–10 seconds | Own design using received timing | Bounds silence after an unlaunched order is rejected or lost. Launch facts take precedence at expiry. Explicit script once requests have no native lease. Discarded host turns restore the previous continuation checkpoint. |
| Missing/nil GetDesire falls back; explicit empty Think overrides; MinionThink-only files retain hero defaults; explicit minion hooks replace courier defaults | Inferred compatibility contract | Implements the host's documented composition of scripts and native policy. Not verified against the closed engine. |
| Engine player ids: Radiant 0–4, Dire 5–9; rank from the received roster, only a proven contiguous prefix before that roster | Inferred compatibility contract | Distinct from wire seat ids. No live-engine mapping verification is claimed. |
| BOT_MODE ids 1–22 in `team::BOT_MODES` order: laning, attack, roam, retreat, secret_shop, side_shop, push_top/mid/bot, defend_top/mid/bot, assemble, team_roam, farm, defend_ally, evasive_maneuvers, roshan, item, ward, rune, outpost | Inferred compatibility contract | Host constants pinned to this order; original engine constants remain to be verified. |
| Engine lane TOP=1/MID=2/BOT=3 mapped to core MID=0/TOP=1/BOT=2 | Inferred engine ids; sourced core ids | Typed conversion pins the distinction; the pin is not engine evidence. |
| GetAttackPoint base seconds; GetAttackSpeed normalized multiplier | Inferred compatibility contract | Documented in API.md; conflicting script usage requires live-engine verification. |

The inferred contracts and unsourced policy choices require verification or an
explicit decision to retain an own-design policy before claiming engine parity.


Embedding measurements and recorded runs
---------------------------------------
`bota_botscript --hash-keys A,B --rng-seed N` fixes both interpreter streams; the used
keys and seed are always printed before connection or source loading. What supplying them
guarantees is under "Determinism" above.
`--summary-json PATH` opts into bounded per-turn fuel and per-thread CPU histograms and
catalogue API counters. Histograms report exact min/max and base-two bucket upper bounds for
median/p99. No timing measurements or API wrappers are installed for ordinary play.
CPU splitting distinguishes snapshot capture, unit/ability handle construction, API work
(excluding that construction), and other script execution. Fuel includes interpreter and host
charges; it is not a count of bytecodes. On Linux CPU samples use the monotonic thread CPU clock.
`--record-views PATH` records the first joined seat's filtered message stream using the existing
replay framing. `--replay-in PATH --seat N --hash-keys A,B --rng-seed N` re-feeds it synchronously
without a server, networking or wall deadlines. Use the same selection policy, source and binary.
Fogless server recordings are refused: they do not retain the original seat observations.
Both modes can write the same summary. Tick lists retain their first 256 entries with a total;
API names are limited to 512 installed names including one other bucket. The first error retains
at most 1024 Unicode characters and JSON encoding escapes control characters. Summary output is
written beside its requested destination then atomically renamed; paths never come from scripts.

Summary measurement costs about half a microsecond per API call in a small local measurement.
It can change wall-deadline discard timing: measured live games are not strength-comparable with
unmeasured games. Supplying both keys and seed selects deterministic fuel-only execution in either
case. Recorded views are atomically published only on success, capped at 1 GiB and 1,048,576 frames;
a cap or write failure stops the run with an error and removes the temporary recording.
Re-feed requires the exact current wire fingerprint and refuses simultaneous recording.

Internal native policy and decoder callbacks are timed under the fixed `other` bucket, so their
work is not attributed to the interpreter. Discard ticks include script-failed outboxes as well
as late results; a delayed failure and its late event count as one recent discarded turn.
The instruction measure is fuel. `Lua::memory()` already supplies currently accounted heap bytes;
additional allocation and collector statistics belong in the independent interpreter embedding API.
